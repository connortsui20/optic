//! Persists immutable captures and serves byte-range queries.
//!
//! [`Store`] owns `SQLite` schema details and content-addressed blobs. Callers see opaque IDs and
//! typed views. A capture becomes visible only after every blob is durable and one catalog
//! transaction commits.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cargo_ir::{EvidenceBundle, LlvmStage, Toolchain};
use fs2::FileExt;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::source::{SourceBaseline, StoredSource};
use crate::{
    BodyView, BuildSpec, CaptureId, CaptureSummary, CompilerOutput, Error, FindResult, InstanceId,
    InstanceSummary, Result, ShowView,
};

const STORE_VERSION: u32 = 4;

pub(crate) struct Store {
    root: PathBuf,
    blobs: PathBuf,
    staging: PathBuf,
    connection: Connection,
}

pub(crate) struct FileLock {
    /// The operating system releases the lock when this file is dropped.
    _file: File,
}

/// Prevents cache removal while a command uses the workspace store.
pub(crate) fn lock_workspace_shared(workspace_root: &Path) -> Result<FileLock> {
    let path = workspace_root.join(".optic.lock");
    let file = open_lock_file(&path)?;
    FileExt::lock_shared(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

    Ok(FileLock { _file: file })
}

/// Waits for active commands and prevents new commands from opening the workspace store.
pub(crate) fn lock_workspace_exclusive(workspace_root: &Path) -> Result<FileLock> {
    let path = workspace_root.join(".optic.lock");
    let file = open_lock_file(&path)?;
    FileExt::lock_exclusive(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

    Ok(FileLock { _file: file })
}

impl Store {
    pub(crate) fn open(workspace_root: &Path) -> Result<Self> {
        let root = workspace_root.join(".optic");
        let blobs = root.join("blobs");
        let staging = root.join("staging");
        let locks = root.join("locks");

        for directory in [&root, &blobs, &staging, &locks] {
            create_private_directory(directory)?;
        }

        let _schema_lock = lock_file(&locks.join("schema.lock"))?;
        let mut connection = Connection::open(root.join("catalog.sqlite"))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        initialize_schema(&mut connection)?;

        Ok(Self {
            root,
            blobs,
            staging,
            connection,
        })
    }

    pub(crate) fn staging_directory(&self, capture_id: &CaptureId) -> PathBuf {
        self.staging.join(capture_id.as_str())
    }

    pub(crate) fn lock_writer(&self) -> Result<FileLock> {
        let path = self.root.join("locks").join("writer.lock");

        lock_file(&path)
    }

    pub(crate) fn cached_capture(&self, request_key: &str) -> Result<Option<CaptureSummary>> {
        let capture_id = self
            .connection
            .query_row(
                "SELECT capture_id FROM capture_cache WHERE request_key = ?1",
                [request_key],
                |row| row.get::<_, CaptureId>(0),
            )
            .optional()?;

        capture_id
            .map(|capture_id| self.capture_summary(&capture_id, true))
            .transpose()
    }

    pub(crate) fn publish(
        &mut self,
        capture_id: &CaptureId,
        request_key: &str,
        spec: &BuildSpec,
        bundle: &EvidenceBundle,
        sources: &SourceBaseline,
        target: &str,
    ) -> Result<CaptureSummary> {
        let created_at_ms = now_ms()?;
        let request_json = serde_json::to_string(spec)?;
        let mut modules = Vec::with_capacity(bundle.modules.len());

        for module in &bundle.modules {
            let bitcode_blob = self.publish_blob(&module.bitcode_path)?;
            let text_blob = self.publish_blob(&module.text_path)?;
            modules.push(PublishedModule {
                name: module.name.clone(),
                stage: module.stage,
                bitcode_blob,
                text_blob,
                bodies: module.bodies.clone(),
            });
        }

        let mut published_sources = Vec::with_capacity(sources.entries.len());
        for source in &sources.entries {
            let blob = self.publish_blob(&source.snapshot)?;
            published_sources.push(PublishedSource {
                path: source.path.to_string_lossy().into_owned(),
                blob,
            });
        }

        let transaction = self.connection.transaction()?;
        insert_capture(
            &transaction,
            capture_id,
            request_key,
            &request_json,
            &bundle.toolchain,
            target,
            created_at_ms,
        )?;
        let body_index = insert_modules(&transaction, capture_id, &modules)?;
        insert_instances(&transaction, capture_id, &bundle.instances, &body_index)?;
        insert_sources(&transaction, capture_id, &published_sources)?;
        transaction.execute(
            "INSERT INTO capture_cache(request_key, capture_id) VALUES (?1, ?2)
             ON CONFLICT(request_key) DO UPDATE SET capture_id = excluded.capture_id",
            params![request_key, capture_id.as_str()],
        )?;
        transaction.commit()?;

        self.capture_summary(capture_id, false)
    }

    pub(crate) fn captures(&self) -> Result<Vec<CaptureSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT id, created_at_ms, rustc_release, llvm_version, target,
                    (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id)
             FROM captures ORDER BY created_at_ms DESC",
        )?;
        let captures = statement
            .query_map([], |row| summary_from_row(row, false))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(captures)
    }

    pub(crate) fn unique_capture_prefix(&self, capture_id: &CaptureId) -> Result<CaptureId> {
        let prefix = self.shortest_unique_prefix(
            capture_id.as_str(),
            "SELECT
                 (SELECT id FROM captures WHERE id < ?1 ORDER BY id DESC LIMIT 1),
                 (SELECT id FROM captures WHERE id > ?1 ORDER BY id LIMIT 1)",
        )?;

        Ok(prefix
            .parse()
            .expect("the capture prefix comes from a validated stored capture ID"))
    }

    pub(crate) fn unique_instance_prefix(&self, instance_id: &InstanceId) -> Result<InstanceId> {
        let prefix = self.shortest_unique_prefix(
            instance_id.as_str(),
            "SELECT
                 (SELECT id FROM instances WHERE id < ?1 ORDER BY id DESC LIMIT 1),
                 (SELECT id FROM instances WHERE id > ?1 ORDER BY id LIMIT 1)",
        )?;

        Ok(prefix
            .parse()
            .expect("the instance prefix comes from a validated stored instance ID"))
    }

    pub(crate) fn find(&self, capture_prefix: &CaptureId, query: &str) -> Result<FindResult> {
        let capture_id = self.resolve_capture(capture_prefix)?;
        let exact = self.query_instances(&capture_id, InstanceMatch::Exact, query)?;
        let instances = if exact.is_empty() {
            self.query_instances(&capture_id, InstanceMatch::Substring, query)?
        } else {
            exact
        };

        Ok(FindResult {
            capture_id,
            instances,
        })
    }

    pub(crate) fn show(
        &self,
        instance_prefix: &InstanceId,
        output: CompilerOutput,
    ) -> Result<ShowView> {
        let resolved = self.resolve_instance(instance_prefix)?;
        let instance = self.connection.query_row(
            "SELECT id, definition, display_name, has_body
             FROM instances WHERE id = ?1",
            [resolved.instance_id.as_str()],
            instance_from_row,
        )?;
        let mut statement = self.connection.prepare(
            "SELECT modules.name, bodies.symbol, modules.text_blob,
                    bodies.start, bodies.end
             FROM bodies
             JOIN modules ON modules.id = bodies.module_id
             WHERE bodies.instance_id = ?1 AND modules.stage = ?2
             ORDER BY modules.name, bodies.start",
        )?;
        let rows = statement.query_map(
            params![resolved.instance_id.as_str(), output.stage().as_str()],
            stored_body_from_row,
        )?;
        let mut bodies = Vec::new();

        for row in rows {
            let body = row?;
            let text = self.read_blob_range(&body.text_blob, body.start, body.end)?;
            bodies.push(BodyView {
                stage: output.stage(),
                module: body.module,
                symbol: body.symbol,
                text,
            });
        }

        Ok(ShowView {
            capture_id: resolved.capture_id,
            instance,
            output,
            bodies,
            source: None,
        })
    }

    pub(crate) fn sources(&self, capture_id: &CaptureId) -> Result<Vec<StoredSource>> {
        let mut statement = self
            .connection
            .prepare("SELECT path, blob FROM sources WHERE capture_id = ?1 ORDER BY path")?;
        let rows = statement.query_map([capture_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut sources = Vec::new();

        for row in rows {
            let (path, blob) = row?;
            let blob_path = self.blob_path(&blob);
            let bytes = fs::read(&blob_path)
                .map_err(|source| Error::filesystem("read", &blob_path, source))?;
            sources.push(StoredSource { path, bytes });
        }

        Ok(sources)
    }

    fn publish_blob(&self, source: &Path) -> Result<String> {
        let digest = hash_file(source)?.to_hex().to_string();
        let destination = self.blob_path(&digest);

        if destination.is_file() {
            return Ok(digest);
        }

        let parent = destination
            .parent()
            .expect("blob paths always contain their two-character digest directory");
        create_private_directory(parent)?;
        let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::now_v7().simple()));
        fs::copy(source, &temporary)
            .map_err(|error| Error::filesystem("copy", &temporary, error))?;

        match fs::rename(&temporary, &destination) {
            Ok(()) => {}
            // Retain a completed blob if another process published the same content first.
            Err(_source) if destination.is_file() => {
                fs::remove_file(&temporary)
                    .map_err(|source| Error::filesystem("remove", &temporary, source))?;
            }
            Err(error) => return Err(Error::filesystem("publish", &destination, error)),
        }

        Ok(digest)
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        let prefix = digest.get(..2).unwrap_or("00");
        self.blobs.join(prefix).join(digest)
    }

    fn capture_summary(&self, capture_id: &CaptureId, reused: bool) -> Result<CaptureSummary> {
        self.connection
            .query_row(
                "SELECT id, created_at_ms, rustc_release, llvm_version, target,
                        (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id)
                 FROM captures WHERE id = ?1",
                [capture_id.as_str()],
                |row| summary_from_row(row, reused),
            )
            .optional()?
            .ok_or_else(|| Error::UnknownCapture {
                capture_id: capture_id.clone(),
            })
    }

    fn resolve_capture(&self, prefix: &CaptureId) -> Result<CaptureId> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM captures
             WHERE id >= ?1 AND id < ?2
             ORDER BY id LIMIT 2",
        )?;
        let upper_bound = hexadecimal_prefix_upper_bound(prefix.as_str());
        let candidates = statement
            .query_map(params![prefix.as_str(), upper_bound], |row| {
                row.get::<_, CaptureId>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        match candidates.as_slice() {
            [] => Err(Error::UnknownCapture {
                capture_id: prefix.clone(),
            }),
            [capture_id] => Ok(capture_id.clone()),
            _ => {
                let capture_ids = candidates
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();

                Err(ambiguous_identifier(
                    "capture",
                    prefix.as_str(),
                    &capture_ids,
                ))
            }
        }
    }

    fn resolve_instance(&self, prefix: &InstanceId) -> Result<ResolvedInstance> {
        let mut statement = self.connection.prepare(
            "SELECT capture_id, id FROM instances
             WHERE id >= ?1 AND id < ?2
             ORDER BY id LIMIT 2",
        )?;
        let upper_bound = hexadecimal_prefix_upper_bound(prefix.as_str());
        let candidates = statement
            .query_map(params![prefix.as_str(), upper_bound], |row| {
                Ok((row.get::<_, CaptureId>(0)?, row.get::<_, InstanceId>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        match candidates.as_slice() {
            [] => Err(Error::UnknownInstance {
                instance_id: prefix.clone(),
            }),
            [(capture_id, instance_id)] => Ok(ResolvedInstance {
                capture_id: capture_id.clone(),
                instance_id: instance_id.clone(),
            }),
            _ => {
                let instance_ids = candidates
                    .iter()
                    .map(|(_, instance_id)| instance_id.to_string())
                    .collect::<Vec<_>>();

                Err(ambiguous_identifier(
                    "instance",
                    prefix.as_str(),
                    &instance_ids,
                ))
            }
        }
    }

    fn shortest_unique_prefix(&self, identifier: &str, neighbors_sql: &str) -> Result<String> {
        let (previous, next) = self
            .connection
            .query_row(neighbors_sql, [identifier], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })?;
        let length = unique_prefix_length(identifier, previous.as_deref(), next.as_deref());

        Ok(identifier[..length].to_owned())
    }

    fn query_instances(
        &self,
        capture_id: &CaptureId,
        match_kind: InstanceMatch,
        query: &str,
    ) -> Result<Vec<InstanceSummary>> {
        let predicate = match match_kind {
            InstanceMatch::Exact => "definition = ?2 OR display_name = ?2",
            InstanceMatch::Substring => "instr(definition, ?2) > 0 OR instr(display_name, ?2) > 0",
        };
        let sql = format!(
            "SELECT id, definition, display_name, has_body FROM instances
             WHERE capture_id = ?1 AND ({predicate}) ORDER BY display_name"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let instances = statement
            .query_map(params![capture_id.as_str(), query], instance_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(instances)
    }

    fn read_blob_range(&self, digest: &str, start: i64, end: i64) -> Result<String> {
        let path = self.blob_path(digest);
        let start = u64::try_from(start).map_err(|_| Error::InvalidRange {
            path: path.clone(),
            start: 0,
            end: 0,
        })?;
        let end = u64::try_from(end).map_err(|_| Error::InvalidRange {
            path: path.clone(),
            start,
            end: 0,
        })?;
        let length = end.checked_sub(start).ok_or_else(|| Error::InvalidRange {
            path: path.clone(),
            start,
            end,
        })?;
        let length = usize::try_from(length).map_err(|_| Error::InvalidRange {
            path: path.clone(),
            start,
            end,
        })?;
        let mut file =
            File::open(&path).map_err(|source| Error::filesystem("open", &path, source))?;
        file.seek(SeekFrom::Start(start))
            .map_err(|source| Error::filesystem("seek", &path, source))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)
            .map_err(|source| Error::filesystem("read", &path, source))?;

        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn hash_file(path: &Path) -> Result<blake3::Hash> {
    let mut file = File::open(path).map_err(|source| Error::filesystem("open", path, source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|source| Error::filesystem("read", path, source))?;

        if length == 0 {
            return Ok(hasher.finalize());
        }

        hasher.update(&buffer[..length]);
    }
}

fn lock_file(path: &Path) -> Result<FileLock> {
    let file = open_lock_file(path)?;
    FileExt::lock_exclusive(&file).map_err(|source| Error::filesystem("lock", path, source))?;

    Ok(FileLock { _file: file })
}

fn open_lock_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| Error::filesystem("open", path, source))
}

fn ambiguous_identifier(kind: &'static str, prefix: &str, candidates: &[String]) -> Error {
    Error::AmbiguousIdentifier {
        kind,
        prefix: prefix.to_owned(),
        candidates: candidates.join(", "),
    }
}

/// Returns the exclusive upper bound for IDs that start with a hexadecimal prefix.
///
/// `g` is the first ASCII character after the largest valid lowercase hexadecimal character.
fn hexadecimal_prefix_upper_bound(prefix: &str) -> String {
    format!("{prefix}g")
}

/// Uses the adjacent sorted IDs because every ID with a shared prefix is contiguous.
fn unique_prefix_length(identifier: &str, previous: Option<&str>, next: Option<&str>) -> usize {
    let minimum_length = identifier.find('_').map_or(1, |index| index + 2);
    let shared_length = [previous, next]
        .into_iter()
        .flatten()
        .map(|neighbor| common_prefix_length(identifier, neighbor))
        .max()
        .unwrap_or(0);

    minimum_length
        .max(shared_length.saturating_add(1))
        .min(identifier.len())
}

fn common_prefix_length(left: &str, right: &str) -> usize {
    left.bytes()
        .zip(right.bytes())
        .take_while(|(left, right)| left == right)
        .count()
}

struct PublishedModule {
    name: String,
    stage: LlvmStage,
    bitcode_blob: String,
    text_blob: String,
    bodies: Vec<cargo_ir::BodyRange>,
}

struct PublishedSource {
    path: String,
    blob: String,
}

struct IndexedBody {
    module_id: String,
    symbol: String,
    start: u64,
    end: u64,
}

struct StoredBody {
    module: String,
    symbol: String,
    text_blob: String,
    start: i64,
    end: i64,
}

struct ResolvedInstance {
    capture_id: CaptureId,
    instance_id: InstanceId,
}

#[derive(Clone, Copy)]
enum InstanceMatch {
    Exact,
    Substring,
}

fn initialize_schema(connection: &mut Connection) -> Result<()> {
    let version =
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))?;

    match version {
        0 => create_schema(connection),
        1 => migrate_schema_v1(connection),
        2 => migrate_schema_v2(connection),
        3 => migrate_schema_v3(connection),
        STORE_VERSION => Ok(()),
        actual => Err(Error::StoreVersion {
            expected: STORE_VERSION,
            actual,
        }),
    }
}

fn create_schema(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE captures(
             id TEXT PRIMARY KEY,
             created_at_ms INTEGER NOT NULL,
             request_key TEXT NOT NULL,
             request_json TEXT NOT NULL,
             rustc_release TEXT NOT NULL,
             rustc_commit TEXT NOT NULL,
             llvm_version TEXT NOT NULL,
             target TEXT NOT NULL,
             profile TEXT NOT NULL
         );
         CREATE TABLE modules(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id),
             name TEXT NOT NULL,
             stage TEXT NOT NULL,
             bitcode_blob TEXT NOT NULL,
             text_blob TEXT NOT NULL
         );
         CREATE TABLE instances(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id),
             definition TEXT NOT NULL,
             display_name TEXT NOT NULL,
             compiler_symbol TEXT,
             has_body INTEGER NOT NULL
         );
         CREATE INDEX instances_definition ON instances(capture_id, definition);
         CREATE INDEX instances_display_name ON instances(capture_id, display_name);
         CREATE TABLE bodies(
             id TEXT PRIMARY KEY,
             instance_id TEXT NOT NULL REFERENCES instances(id),
             module_id TEXT NOT NULL REFERENCES modules(id),
             symbol TEXT NOT NULL,
             start INTEGER NOT NULL,
             end INTEGER NOT NULL
         );
         CREATE INDEX bodies_instance ON bodies(instance_id);
         CREATE TABLE sources(
             capture_id TEXT NOT NULL REFERENCES captures(id),
             path TEXT NOT NULL,
             blob TEXT NOT NULL,
             PRIMARY KEY(capture_id, path)
         );
         CREATE TABLE capture_cache(
             request_key TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id)
         );",
    )?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    transaction.commit()?;

    Ok(())
}

fn migrate_schema_v1(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    let has_capture_cache = schema_object_exists(&transaction, "table", "capture_cache")?;
    let has_analysis_cache = schema_object_exists(&transaction, "table", "analysis_cache")?;

    if !has_capture_cache {
        if has_analysis_cache {
            transaction.execute_batch("ALTER TABLE analysis_cache RENAME TO capture_cache;")?;
        } else {
            transaction.execute_batch(
                "CREATE TABLE capture_cache(
                     request_key TEXT PRIMARY KEY,
                     capture_id TEXT NOT NULL REFERENCES captures(id)
                 );",
            )?;
        }
    }

    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS instances_display_name
         ON instances(capture_id, display_name);
         CREATE INDEX IF NOT EXISTS bodies_instance ON bodies(instance_id);
         ALTER TABLE instances ADD COLUMN compiler_symbol TEXT;",
    )?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    transaction.commit()?;

    Ok(())
}

fn migrate_schema_v2(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE INDEX instances_display_name ON instances(capture_id, display_name);
         CREATE INDEX bodies_instance ON bodies(instance_id);
         ALTER TABLE instances ADD COLUMN compiler_symbol TEXT;",
    )?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    transaction.commit()?;

    Ok(())
}

fn migrate_schema_v3(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch("ALTER TABLE instances ADD COLUMN compiler_symbol TEXT;")?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    transaction.commit()?;

    Ok(())
}

fn schema_object_exists(connection: &Connection, object_type: &str, name: &str) -> Result<bool> {
    let exists = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema WHERE type = ?1 AND name = ?2
         )",
        [object_type, name],
        |row| row.get(0),
    )?;

    Ok(exists)
}

fn insert_capture(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    request_key: &str,
    request_json: &str,
    toolchain: &Toolchain,
    target: &str,
    created_at_ms: u64,
) -> Result<()> {
    let created_at_ms = sqlite_integer("capture creation time", created_at_ms)?;

    transaction.execute(
        "INSERT INTO captures(
             id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
             llvm_version, target, profile
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'enriched')",
        params![
            capture_id.as_str(),
            created_at_ms,
            request_key,
            request_json,
            toolchain.release,
            toolchain.commit_hash,
            toolchain.llvm_version,
            target,
        ],
    )?;

    Ok(())
}

fn insert_modules(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    modules: &[PublishedModule],
) -> Result<HashMap<String, Vec<IndexedBody>>> {
    let mut body_index: HashMap<String, Vec<IndexedBody>> = HashMap::new();

    for module in modules {
        let module_id = format!("mod_{}", uuid::Uuid::now_v7().simple());
        transaction.execute(
            "INSERT INTO modules(id, capture_id, name, stage, bitcode_blob, text_blob)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                module_id,
                capture_id.as_str(),
                module.name,
                module.stage.as_str(),
                module.bitcode_blob,
                module.text_blob,
            ],
        )?;

        for body in &module.bodies {
            body_index
                .entry(body.raw_symbol.clone())
                .or_default()
                .push(IndexedBody {
                    module_id: module_id.clone(),
                    symbol: body.raw_symbol.clone(),
                    start: body.start,
                    end: body.end,
                });
        }
    }

    Ok(body_index)
}

fn insert_instances(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    instances: &[cargo_ir::CompilerInstance],
    body_index: &HashMap<String, Vec<IndexedBody>>,
) -> Result<()> {
    for instance in instances {
        let instance_id = InstanceId::new();
        let bodies = bodies_for_instance(instance, body_index);
        transaction.execute(
            "INSERT INTO instances(
                 id, capture_id, definition, display_name, compiler_symbol, has_body
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                instance_id.as_str(),
                capture_id.as_str(),
                instance.definition,
                instance.display_name,
                instance.raw_symbol,
                !bodies.is_empty(),
            ],
        )?;

        for body in bodies {
            let start = sqlite_integer("LLVM body start offset", body.start)?;
            let end = sqlite_integer("LLVM body end offset", body.end)?;

            transaction.execute(
                "INSERT INTO bodies(id, instance_id, module_id, symbol, start, end)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    format!("body_{}", uuid::Uuid::now_v7().simple()),
                    instance_id.as_str(),
                    body.module_id,
                    body.symbol,
                    start,
                    end,
                ],
            )?;
        }
    }

    Ok(())
}

fn bodies_for_instance<'a>(
    instance: &cargo_ir::CompilerInstance,
    body_index: &'a HashMap<String, Vec<IndexedBody>>,
) -> &'a [IndexedBody] {
    body_index
        .get(&instance.raw_symbol)
        .map_or(&[], Vec::as_slice)
}

fn insert_sources(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    sources: &[PublishedSource],
) -> Result<()> {
    for source in sources {
        transaction.execute(
            "INSERT INTO sources(capture_id, path, blob) VALUES (?1, ?2, ?3)",
            params![capture_id.as_str(), source.path, source.blob],
        )?;
    }

    Ok(())
}

fn summary_from_row(row: &rusqlite::Row<'_>, reused: bool) -> rusqlite::Result<CaptureSummary> {
    Ok(CaptureSummary {
        id: row.get(0)?,
        created_at_ms: integer_from_row(row, 1)?,
        reused,
        rustc_release: row.get(2)?,
        llvm_version: row.get(3)?,
        target: row.get(4)?,
        instance_count: integer_from_row(row, 5)?,
    })
}

fn integer_from_row<T>(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<T>
where
    T: TryFrom<i64>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    let value = row.get::<_, i64>(index)?;

    T::try_from(value).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Integer, Box::new(source))
    })
}

fn instance_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstanceSummary> {
    Ok(InstanceSummary {
        id: row.get(0)?,
        definition: row.get(1)?,
        display_name: row.get(2)?,
        has_body: row.get(3)?,
    })
}

fn stored_body_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredBody> {
    Ok(StoredBody {
        module: row.get(0)?,
        symbol: row.get(1)?,
        text_blob: row.get(2)?,
        start: row.get(3)?,
        end: row.get(4)?,
    })
}

fn now_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| Error::SystemClock { source })?;

    let milliseconds = duration.as_millis();

    u64::try_from(milliseconds).map_err(|_| Error::IntegerOutOfRange {
        name: "Unix time in milliseconds",
        maximum: u64::MAX.into(),
        actual: milliseconds,
    })
}

fn sqlite_integer(name: &'static str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::IntegerOutOfRange {
        name,
        maximum: i64::MAX as u128,
        actual: value.into(),
    })
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::filesystem("create", path, source))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, permissions)
            .map_err(|source| Error::filesystem("set permissions on", path, source))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cargo_ir::CompilerInstance;
    use rusqlite::Connection;

    use super::{
        IndexedBody, STORE_VERSION, Store, bodies_for_instance, schema_object_exists,
        unique_prefix_length,
    };
    use crate::{CaptureId, Error, InstanceId};

    #[test]
    fn associates_an_instance_only_with_its_exact_compiler_symbol() {
        let instance = CompilerInstance {
            definition: "mask_iteration::for_each_set_index".to_owned(),
            display_name: "mask_iteration::for_each_set_index".to_owned(),
            raw_symbol: "_Rmask_iteration".to_owned(),
            codegen_units: vec![
                "mask_iteration.abc-cgu.00".to_owned(),
                "mask_iteration.abc-cgu.08".to_owned(),
            ],
        };
        let mut body_index = HashMap::new();
        body_index.insert(
            "_Rmask_iteration".to_owned(),
            vec![IndexedBody {
                module_id: "module".to_owned(),
                symbol: "symbol".to_owned(),
                start: 0,
                end: 1,
            }],
        );

        let bodies = bodies_for_instance(&instance, &body_index);

        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].symbol, "symbol");
    }

    #[test]
    fn does_not_associate_an_llvm_clone_by_its_display_name() {
        let instance = CompilerInstance {
            definition: "mask_iteration::make".to_owned(),
            display_name: "mask_iteration::make".to_owned(),
            raw_symbol: "_Rmake".to_owned(),
            codegen_units: vec!["mask_iteration.abc-cgu.00".to_owned()],
        };
        let body_index = HashMap::from([(
            "_Rmake.llvm.123".to_owned(),
            vec![IndexedBody {
                module_id: "module".to_owned(),
                symbol: "_Rmake.llvm.123".to_owned(),
                start: 0,
                end: 1,
            }],
        )]);

        assert!(bodies_for_instance(&instance, &body_index).is_empty());
    }

    #[test]
    fn includes_one_character_after_the_longest_neighbor_match() {
        assert_eq!(
            unique_prefix_length("ins_01234567", Some("ins_0122ffff"), Some("ins_01239abc")),
            9
        );
        assert_eq!(unique_prefix_length("cap_abcdef", None, None), 5);
    }

    #[test]
    fn migrates_the_analysis_cache_table_from_schema_v1() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can create a current store");
        store
            .connection
            .execute_batch(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
                     llvm_version, target, profile
                 ) VALUES (
                     'cap_0123456789abcdef0123456789abcdef', 0, 'request', '{}',
                     'nightly', '', 'LLVM', 'target', 'release'
                 );
                 INSERT INTO capture_cache(request_key, capture_id) VALUES (
                     'request', 'cap_0123456789abcdef0123456789abcdef'
                 );",
            )
            .expect("the test can insert cached evidence");
        drop(store);

        let catalog = temporary.path().join(".optic/catalog.sqlite");
        let connection = Connection::open(&catalog).expect("the test can open the catalog");
        connection
            .execute_batch(
                "DROP INDEX instances_display_name;
                 DROP INDEX bodies_instance;
                 ALTER TABLE instances DROP COLUMN compiler_symbol;
                 ALTER TABLE capture_cache RENAME TO analysis_cache;
                 PRAGMA user_version = 1;",
            )
            .expect("the test can create the version 1 layout");
        drop(connection);

        let store = Store::open(temporary.path()).expect("the store can migrate schema version 1");
        let version = store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .expect("the migrated schema has a version");
        let cached = store
            .cached_capture("request")
            .expect("the migrated cache is readable")
            .expect("the migration preserves the cached capture");

        assert_eq!(version, STORE_VERSION);
        assert_eq!(cached.id.as_str(), "cap_0123456789abcdef0123456789abcdef");
        assert!(
            schema_object_exists(&store.connection, "table", "capture_cache")
                .expect("the test can inspect the migrated schema")
        );
        assert!(
            !schema_object_exists(&store.connection, "table", "analysis_cache")
                .expect("the test can inspect the migrated schema")
        );
        assert!(
            schema_object_exists(&store.connection, "index", "instances_display_name")
                .expect("the test can inspect the migrated schema")
        );
        assert!(
            schema_object_exists(&store.connection, "index", "bodies_instance")
                .expect("the test can inspect the migrated schema")
        );
        assert!(column_exists(
            &store.connection,
            "instances",
            "compiler_symbol"
        ));
    }

    #[test]
    fn adds_query_indexes_to_schema_v2() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can create a current store");
        store
            .connection
            .execute_batch(
                "DROP INDEX instances_display_name;
                 DROP INDEX bodies_instance;
                 ALTER TABLE instances DROP COLUMN compiler_symbol;
                 PRAGMA user_version = 2;",
            )
            .expect("the test can create the version 2 layout");
        drop(store);

        let store = Store::open(temporary.path()).expect("the store can migrate schema version 2");
        let version = store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .expect("the migrated schema has a version");

        assert_eq!(version, STORE_VERSION);
        assert!(
            schema_object_exists(&store.connection, "index", "instances_display_name")
                .expect("the test can inspect the migrated schema")
        );
        assert!(
            schema_object_exists(&store.connection, "index", "bodies_instance")
                .expect("the test can inspect the migrated schema")
        );
        assert!(column_exists(
            &store.connection,
            "instances",
            "compiler_symbol"
        ));
    }

    #[test]
    fn adds_compiler_symbols_to_schema_v3() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can create a current store");
        store
            .connection
            .execute_batch(
                "ALTER TABLE instances DROP COLUMN compiler_symbol;
                 PRAGMA user_version = 3;",
            )
            .expect("the test can create the version 3 layout");
        drop(store);

        let store = Store::open(temporary.path()).expect("the store can migrate schema version 3");
        let version = store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .expect("the migrated schema has a version");

        assert_eq!(version, STORE_VERSION);
        assert!(column_exists(
            &store.connection,
            "instances",
            "compiler_symbol"
        ));
    }

    #[test]
    fn rejects_ambiguous_capture_and_instance_prefixes() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = super::Store::open(temporary.path()).expect("the test can open a store");
        store
            .connection
            .execute_batch(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
                     llvm_version, target, profile
                 ) VALUES
                     ('cap_00000000000000000000000000000000', 0, 'a', '{}', '', '', '', '', ''),
                     ('cap_0fffffffffffffffffffffffffffffff', 0, 'b', '{}', '', '', '', '', '');
                 INSERT INTO instances(id, capture_id, definition, display_name, has_body) VALUES
                     ('ins_00000000000000000000000000000000',
                      'cap_00000000000000000000000000000000', '', '', 0),
                     ('ins_0fffffffffffffffffffffffffffffff',
                      'cap_0fffffffffffffffffffffffffffffff', '', '', 0);",
            )
            .expect("the test can insert colliding IDs");

        let capture_prefix = "cap_0"
            .parse::<CaptureId>()
            .expect("the capture prefix is valid");
        let instance_prefix = "ins_0"
            .parse::<InstanceId>()
            .expect("the instance prefix is valid");

        assert!(matches!(
            store.resolve_capture(&capture_prefix),
            Err(Error::AmbiguousIdentifier {
                kind: "capture",
                ..
            })
        ));
        assert!(matches!(
            store.resolve_instance(&instance_prefix),
            Err(Error::AmbiguousIdentifier {
                kind: "instance",
                ..
            })
        ));
    }

    #[test]
    fn returns_an_error_for_an_invalid_stored_identifier() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        store
            .connection
            .execute(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
                     llvm_version, target, profile
                 ) VALUES ('invalid', 0, 'request', '{}', '', '', '', '', '')",
                [],
            )
            .expect("the test can insert an invalid stored identifier");

        assert!(matches!(store.captures(), Err(Error::Database(_))));
    }

    #[track_caller]
    fn column_exists(connection: &Connection, table: &str, column: &str) -> bool {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("the test can inspect table columns");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("the test can query table columns")
            .any(|name| name.is_ok_and(|name| name == column))
    }
}
