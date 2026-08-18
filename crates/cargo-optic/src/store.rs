//! Persists immutable captures and serves byte-range queries.
//!
//! [`Store`] owns SQLite schema details and content-addressed blobs. Callers see opaque IDs and
//! typed views. A capture becomes visible only after every blob is durable and one catalog
//! transaction commits.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cargo_ir::{EvidenceBundle, Toolchain};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::source::{SourceBaseline, StoredSource};
use crate::{
    BodyView, BuildSpec, CaptureId, CaptureSummary, Error, FindResult, InstanceId, InstanceSummary,
    Result, ShowView,
};

const STORE_VERSION: u32 = 1;

pub(crate) struct Store {
    root: PathBuf,
    blobs: PathBuf,
    staging: PathBuf,
    connection: Connection,
}

pub(crate) struct FileLock {
    _file: File,
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
        let connection = Connection::open(root.join("catalog.sqlite"))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        initialize_schema(&connection)?;

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
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        capture_id
            .map(|capture_id| self.capture_summary(&capture_id, true))
            .transpose()
    }

    pub(crate) fn publish(
        &mut self,
        capture_id: CaptureId,
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
                stage: module.stage.clone(),
                bitcode_blob,
                text_blob,
                bodies: module.bodies.clone(),
            });
        }

        let mut published_sources = Vec::with_capacity(sources.entries.len());
        for source in &sources.entries {
            let blob = self.publish_blob(&source.snapshot)?;
            published_sources.push((source.path.to_string_lossy().into_owned(), blob));
        }

        let transaction = self.connection.transaction()?;
        insert_capture(
            &transaction,
            &capture_id,
            request_key,
            &request_json,
            &bundle.toolchain,
            target,
            created_at_ms,
        )?;
        let body_index = insert_modules(&transaction, &capture_id, &modules)?;
        insert_instances(&transaction, &capture_id, &bundle.mono_items, &body_index)?;
        insert_sources(&transaction, &capture_id, &published_sources)?;
        transaction.execute(
            "INSERT INTO capture_cache(request_key, capture_id) VALUES (?1, ?2)
             ON CONFLICT(request_key) DO UPDATE SET capture_id = excluded.capture_id",
            params![request_key, capture_id.as_str()],
        )?;
        transaction.commit()?;

        self.capture_summary(capture_id.as_str(), false)
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

    pub(crate) fn find(&self, capture_id: &CaptureId, query: &str) -> Result<FindResult> {
        self.ensure_capture(capture_id)?;
        let exact =
            self.query_instances(capture_id, "definition = ?2 OR display_name = ?2", query)?;
        let instances = if exact.is_empty() {
            self.query_instances(
                capture_id,
                "instr(definition, ?2) > 0 OR instr(display_name, ?2) > 0",
                query,
            )?
        } else {
            exact
        };

        Ok(FindResult {
            capture_id: capture_id.clone(),
            instances,
        })
    }

    pub(crate) fn show(
        &self,
        capture_id: &CaptureId,
        instance_id: &InstanceId,
    ) -> Result<ShowView> {
        let instance = self
            .connection
            .query_row(
                "SELECT id, definition, display_name, has_body
                 FROM instances WHERE id = ?1 AND capture_id = ?2",
                params![instance_id.as_str(), capture_id.as_str()],
                instance_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::UnknownInstance {
                instance_id: instance_id.clone(),
            })?;
        let mut statement = self.connection.prepare(
            "SELECT modules.stage, modules.name, bodies.symbol, modules.text_blob,
                    bodies.start, bodies.end
             FROM bodies
             JOIN modules ON modules.id = bodies.module_id
             WHERE bodies.instance_id = ?1
             ORDER BY modules.stage, modules.name, bodies.start",
        )?;
        let rows = statement.query_map([instance_id.as_str()], |row| {
            Ok(StoredBody {
                stage: row.get(0)?,
                module: row.get(1)?,
                symbol: row.get(2)?,
                text_blob: row.get(3)?,
                start: row.get::<_, i64>(4)?,
                end: row.get::<_, i64>(5)?,
            })
        })?;
        let mut bodies = Vec::new();

        for row in rows {
            let body = row?;
            let text = self.read_blob_range(&body.text_blob, body.start, body.end)?;
            bodies.push(BodyView {
                stage: body.stage,
                module: body.module,
                symbol: body.symbol,
                text,
            });
        }

        Ok(ShowView {
            capture_id: capture_id.clone(),
            instance,
            bodies,
            source: None,
        })
    }

    pub(crate) fn sources(&self, capture_id: &CaptureId) -> Result<Vec<StoredSource>> {
        self.ensure_capture(capture_id)?;
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
            Err(error) if destination.is_file() => {
                fs::remove_file(&temporary)
                    .map_err(|source| Error::filesystem("remove", &temporary, source))?;
                let _ = error;
            }
            Err(error) => return Err(Error::filesystem("publish", &destination, error)),
        }

        Ok(digest)
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        let prefix = digest.get(..2).unwrap_or("00");
        self.blobs.join(prefix).join(digest)
    }

    fn capture_summary(&self, capture_id: &str, reused: bool) -> Result<CaptureSummary> {
        self.connection
            .query_row(
                "SELECT id, created_at_ms, rustc_release, llvm_version, target,
                        (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id)
                 FROM captures WHERE id = ?1",
                [capture_id],
                |row| summary_from_row(row, reused),
            )
            .optional()?
            .ok_or_else(|| Error::UnknownCapture {
                capture_id: capture_id
                    .parse()
                    .expect("stored capture IDs are validated on insert"),
            })
    }

    fn ensure_capture(&self, capture_id: &CaptureId) -> Result<()> {
        let exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM captures WHERE id = ?1)",
            [capture_id.as_str()],
            |row| row.get::<_, bool>(0),
        )?;

        if !exists {
            return Err(Error::UnknownCapture {
                capture_id: capture_id.clone(),
            });
        }

        Ok(())
    }

    fn query_instances(
        &self,
        capture_id: &CaptureId,
        predicate: &str,
        query: &str,
    ) -> Result<Vec<InstanceSummary>> {
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
    let mut buffer = [0_u8; 64 * 1024];

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
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| Error::filesystem("open", path, source))?;
    file.lock_exclusive()
        .map_err(|source| Error::filesystem("lock", path, source))?;

    Ok(FileLock { _file: file })
}

struct PublishedModule {
    name: String,
    stage: String,
    bitcode_blob: String,
    text_blob: String,
    bodies: Vec<cargo_ir::BodyRange>,
}

struct IndexedBody {
    module_id: String,
    symbol: String,
    start: u64,
    end: u64,
}

struct StoredBody {
    stage: String,
    module: String,
    symbol: String,
    text_blob: String,
    start: i64,
    end: i64,
}

fn initialize_schema(connection: &Connection) -> Result<()> {
    let version =
        connection.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))?;

    if version != 0 && version != STORE_VERSION {
        return Err(Error::StoreVersion {
            expected: STORE_VERSION,
            actual: version,
        });
    }
    if version == STORE_VERSION {
        return Ok(());
    }

    connection.execute_batch(
        "BEGIN;
         CREATE TABLE captures(
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
             has_body INTEGER NOT NULL
         );
         CREATE INDEX instances_definition ON instances(capture_id, definition);
         CREATE TABLE bodies(
             id TEXT PRIMARY KEY,
             instance_id TEXT NOT NULL REFERENCES instances(id),
             module_id TEXT NOT NULL REFERENCES modules(id),
             symbol TEXT NOT NULL,
             start INTEGER NOT NULL,
             end INTEGER NOT NULL
         );
         CREATE TABLE sources(
             capture_id TEXT NOT NULL REFERENCES captures(id),
             path TEXT NOT NULL,
             blob TEXT NOT NULL,
             PRIMARY KEY(capture_id, path)
         );
         CREATE TABLE capture_cache(
             request_key TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id)
         );
         PRAGMA user_version = 1;
         COMMIT;",
    )?;

    Ok(())
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
    transaction.execute(
        "INSERT INTO captures(
             id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
             llvm_version, target, profile
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'enriched')",
        params![
            capture_id.as_str(),
            i64::try_from(created_at_ms).unwrap_or(i64::MAX),
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
                module.stage,
                module.bitcode_blob,
                module.text_blob,
            ],
        )?;

        for body in &module.bodies {
            body_index
                .entry(body.demangled.clone())
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
    mono_items: &[cargo_ir::MonoItem],
    body_index: &HashMap<String, Vec<IndexedBody>>,
) -> Result<()> {
    for item in mono_items {
        let instance_id = InstanceId::new();
        let bodies = body_index.get(&item.name).map(Vec::as_slice).unwrap_or(&[]);
        let definition = definition_name(&item.name);
        transaction.execute(
            "INSERT INTO instances(id, capture_id, definition, display_name, has_body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                instance_id.as_str(),
                capture_id.as_str(),
                definition,
                item.name,
                !bodies.is_empty(),
            ],
        )?;

        for body in bodies {
            transaction.execute(
                "INSERT INTO bodies(id, instance_id, module_id, symbol, start, end)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    format!("body_{}", uuid::Uuid::now_v7().simple()),
                    instance_id.as_str(),
                    body.module_id,
                    body.symbol,
                    i64::try_from(body.start).unwrap_or(i64::MAX),
                    i64::try_from(body.end).unwrap_or(i64::MAX),
                ],
            )?;
        }
    }

    Ok(())
}

fn insert_sources(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    sources: &[(String, String)],
) -> Result<()> {
    for (path, blob) in sources {
        transaction.execute(
            "INSERT INTO sources(capture_id, path, blob) VALUES (?1, ?2, ?3)",
            params![capture_id.as_str(), path, blob],
        )?;
    }

    Ok(())
}

fn definition_name(instance: &str) -> String {
    let Some(start) = instance.rfind("::<") else {
        return instance.to_owned();
    };
    let Some(end) = generic_arguments_end(instance, start + 2) else {
        return instance.to_owned();
    };

    format!("{}{}", &instance[..start], &instance[end..])
}

fn generic_arguments_end(instance: &str, start: usize) -> Option<usize> {
    let mut depth = 0_usize;

    for (offset, character) in instance[start..].char_indices() {
        match character {
            '<' => depth += 1,
            '>' => {
                depth = depth.checked_sub(1)?;

                if depth == 0 {
                    return Some(start + offset + character.len_utf8());
                }
            }
            _ => {}
        }
    }

    None
}

fn summary_from_row(row: &rusqlite::Row<'_>, reused: bool) -> rusqlite::Result<CaptureSummary> {
    let id: String = row.get(0)?;
    let created_at_ms: i64 = row.get(1)?;
    let instance_count: i64 = row.get(5)?;

    Ok(CaptureSummary {
        id: id
            .parse()
            .expect("stored capture IDs are validated on insert"),
        created_at_ms: u64::try_from(created_at_ms).unwrap_or_default(),
        reused,
        rustc_release: row.get(2)?,
        llvm_version: row.get(3)?,
        target: row.get(4)?,
        instance_count: usize::try_from(instance_count).unwrap_or(usize::MAX),
    })
}

fn instance_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstanceSummary> {
    let id: String = row.get(0)?;

    Ok(InstanceSummary {
        id: id
            .parse()
            .expect("stored instance IDs are validated on insert"),
        definition: row.get(1)?,
        display_name: row.get(2)?,
        has_body: row.get(3)?,
    })
}

fn now_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| Error::InvalidRequest {
            message: format!("system clock is before the Unix epoch, got {source}"),
        })?;

    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
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
    use super::definition_name;

    #[test]
    fn removes_only_the_final_generic_arguments() {
        assert_eq!(
            definition_name("crate::kernel::<Vec<u64>, 8>"),
            "crate::kernel"
        );
        assert_eq!(
            definition_name("crate::kernel::<Vec<u64>, 8>::{closure#0}"),
            "crate::kernel::{closure#0}"
        );
        assert_eq!(definition_name("crate::plain"), "crate::plain");
    }
}
