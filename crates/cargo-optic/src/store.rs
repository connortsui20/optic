//! Persists immutable captures and serves byte-range queries.
//!
//! [`Store`] owns `SQLite` schema details and content-addressed blobs. Callers see opaque IDs and
//! typed views. A capture becomes visible only after every blob is durable and one catalog
//! transaction commits.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cargo_ir::{EvidenceBundle, LlvmStage, Toolchain};
use fs2::FileExt;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::source::{SourceBaseline, StoredSource};
use crate::{
    ArtifactSummary, BodyView, BuildSpec, CaptureDetails, CaptureId, CaptureProfile,
    CaptureSummary, CommandView, CompilerOutput, EnvironmentView, Error, FindResult, GcSummary,
    InstanceId, InstanceSummary, OutputAvailability, RemoveSummary, Result, ShowView,
    SourceLocation, StoreStatus, VerifySummary,
};

const STORE_VERSION: u32 = 5;

pub(crate) struct Store {
    root: PathBuf,
    blobs: PathBuf,
    staging: PathBuf,
    work: PathBuf,
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
        let work = root.join("work");
        let locks = root.join("locks");

        for directory in [&root, &blobs, &staging, &work, &locks] {
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
            work,
            connection,
        })
    }

    pub(crate) fn staging_directory(&self, capture_id: &CaptureId) -> PathBuf {
        self.staging.join(capture_id.as_str())
    }

    pub(crate) fn analysis_directory(&self, request_key: &str) -> PathBuf {
        debug_assert!(
            request_key.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "request keys are hexadecimal digests"
        );

        self.work.join(request_key)
    }

    pub(crate) fn lock_writer(&self) -> Result<FileLock> {
        let path = self.root.join("locks").join("writer.lock");

        lock_file(&path)
    }

    pub(crate) fn lock_evidence_reader(&self) -> Result<FileLock> {
        let path = self.root.join("locks").join("evidence.lock");
        let file = open_lock_file(&path)?;
        FileExt::lock_shared(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

        Ok(FileLock { _file: file })
    }

    fn lock_evidence_writer(&self) -> Result<FileLock> {
        let path = self.root.join("locks").join("evidence.lock");

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
            .map(|capture_id| {
                self.verify_capture_blobs(&capture_id)?;
                self.capture_summary(&capture_id, true)
            })
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
        let invocation_json = serde_json::to_string(&bundle.invocation)?;
        let mut modules = Vec::with_capacity(bundle.modules.len());

        for module in &bundle.modules {
            let bitcode_blob = self.publish_blob(&module.bitcode_path)?;
            let text_blob = self.publish_blob(&module.text_path)?;
            modules.push(PublishedModule {
                name: module.name.clone(),
                provenance: module.provenance.clone(),
                bitcode_blob,
                text_blob,
                bodies: module.bodies.clone(),
                declarations: module.declarations.clone(),
                aliases: module.aliases.clone(),
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
            PublishedCapture {
                capture_id,
                request_key,
                request_json: &request_json,
                invocation_json: &invocation_json,
                spec,
                toolchain: &bundle.toolchain,
                target,
                created_at_ms,
            },
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
            "SELECT id, created_at_ms, rustc_release, llvm_version, target, profile,
                    (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id),
                    (SELECT COUNT(*) FROM modules WHERE capture_id = captures.id)
             FROM captures ORDER BY created_at_ms DESC",
        )?;
        let captures = statement
            .query_map([], |row| summary_from_row(row, false))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(captures)
    }

    pub(crate) fn status(&self) -> Result<StoreStatus> {
        let captures = self
            .connection
            .query_row("SELECT COUNT(*) FROM captures", [], |row| {
                integer_from_row(row, 0)
            })?;
        let blobs = self.blob_entries()?;

        Ok(StoreStatus {
            captures,
            blobs: blobs.len(),
            blob_bytes: blobs.iter().map(|blob| blob.bytes).sum(),
        })
    }

    pub(crate) fn remove_capture(&mut self, capture_prefix: &CaptureId) -> Result<RemoveSummary> {
        let _evidence = self.lock_evidence_writer()?;
        let capture_id = self.resolve_capture(capture_prefix)?;
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM captures WHERE id = ?1", [capture_id.as_str()])?;
        transaction.commit()?;

        Ok(RemoveSummary { capture_id })
    }

    pub(crate) fn gc(&self) -> Result<GcSummary> {
        let _evidence = self.lock_evidence_writer()?;
        let referenced = self.referenced_blob_digests()?;
        let mut removed_blobs = 0;
        let mut removed_bytes = 0_u64;
        let mut changed_directories = HashSet::new();

        for blob in self.blob_entries()? {
            if referenced.contains(&blob.digest) {
                continue;
            }
            fs::remove_file(&blob.path)
                .map_err(|source| Error::filesystem("remove", &blob.path, source))?;
            removed_blobs += 1;
            removed_bytes = removed_bytes.saturating_add(blob.bytes);
            if let Some(parent) = blob.path.parent() {
                changed_directories.insert(parent.to_owned());
            }
        }
        for directory in changed_directories {
            sync_directory(&directory)?;
        }
        if removed_blobs != 0 {
            sync_directory(&self.blobs)?;
        }

        Ok(GcSummary {
            removed_blobs,
            removed_bytes,
        })
    }

    pub(crate) fn verify(&self) -> Result<VerifySummary> {
        let referenced = self.referenced_blob_digests()?;
        let mut verified_bytes = 0_u64;

        for digest in &referenced {
            let (path, expected) = self.verified_blob_path(digest)?;
            verify_file_digest(&path, expected)?;
            verified_bytes = verified_bytes.saturating_add(
                fs::metadata(&path)
                    .map_err(|source| Error::filesystem("read metadata for", &path, source))?
                    .len(),
            );
        }

        Ok(VerifySummary {
            verified_blobs: referenced.len(),
            verified_bytes,
        })
    }

    pub(crate) fn capture_details(&self, capture_prefix: &CaptureId) -> Result<CaptureDetails> {
        let capture_id = self.resolve_capture(capture_prefix)?;
        let summary = self.capture_summary(&capture_id, false)?;
        let (request_json, invocation_json) = self.connection.query_row(
            "SELECT request_json, invocation_json FROM captures WHERE id = ?1",
            [capture_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let request = serde_json::from_str::<BuildSpec>(&request_json)?;
        let invocation = serde_json::from_str::<cargo_ir::CaptureInvocation>(&invocation_json)?;
        let mut statement = self.connection.prepare(
            "SELECT modules.name, modules.stage, modules.compiler_stage, modules.codegen_unit,
                    modules.lto, modules.capture_method,
                    (SELECT COUNT(*) FROM bodies WHERE module_id = modules.id),
                    (SELECT COUNT(*) FROM declarations WHERE module_id = modules.id),
                    (SELECT COUNT(*) FROM aliases WHERE module_id = modules.id)
             FROM modules WHERE capture_id = ?1 ORDER BY modules.name, modules.compiler_stage",
        )?;
        let artifacts = statement
            .query_map([capture_id.as_str()], artifact_summary_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(CaptureDetails {
            summary,
            request,
            cargo: command_view(invocation.cargo, &self.root),
            rustc: invocation
                .rustc
                .map(|command| command_view(command, &self.root)),
            wrapper_chain: invocation.wrapper_chain,
            environment: invocation
                .environment
                .into_iter()
                .map(|variable| EnvironmentView {
                    name: variable.name,
                    value: variable.value,
                })
                .collect(),
            injected_rustc_arguments: invocation
                .injected_rustc_arguments
                .into_iter()
                .map(|argument| sanitize_store_path(argument, &self.root))
                .collect(),
            artifacts,
        })
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
            &format!("{} WHERE instances.id = ?1", instance_select()),
            [resolved.instance_id.as_str()],
            instance_from_row,
        )?;
        let mut statement = self.connection.prepare(
            "SELECT modules.name, bodies.symbol, modules.text_blob,
                    bodies.start, bodies.end
             FROM bodies
             JOIN instance_bodies ON instance_bodies.body_id = bodies.id
             JOIN selected_modules AS modules ON modules.id = bodies.module_id
             WHERE instance_bodies.instance_id = ?1 AND modules.stage = ?2
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
                summary: crate::LlvmBodySummary::from_text(&text),
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
            let bytes = self.read_blob(&blob)?;
            sources.push(StoredSource { path, bytes });
        }

        Ok(sources)
    }

    pub(crate) fn source_file(
        &self,
        capture_id: &CaptureId,
        location: &SourceLocation,
    ) -> Result<Option<StoredSource>> {
        let mut statement = self
            .connection
            .prepare("SELECT path, blob FROM sources WHERE capture_id = ?1 ORDER BY path")?;
        let candidates = statement
            .query_map([capture_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|row| match row {
                Ok((path, blob)) if source_path_matches(&path, &location.path) => {
                    Some(Ok((path, blob)))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let [(path, blob)] = candidates.as_slice() else {
            return Ok(None);
        };
        let bytes = self.read_blob(blob)?;

        Ok(Some(StoredSource {
            path: path.clone(),
            bytes,
        }))
    }

    fn publish_blob(&self, source: &Path) -> Result<String> {
        let temporary = self
            .blobs
            .join(format!(".{}.tmp", uuid::Uuid::now_v7().simple()));
        let expected_digest = match copy_and_hash_file(source, &temporary) {
            Ok(digest) => digest,
            Err(error) => {
                let _ = fs::remove_file(&temporary);

                return Err(error);
            }
        };
        let digest = expected_digest.to_hex().to_string();
        let destination = self.blob_path(&digest);

        if destination.is_file() {
            let result = verify_file_digest(&destination, expected_digest);
            let _ = fs::remove_file(&temporary);

            result?;

            return Ok(digest);
        }

        let parent = destination
            .parent()
            .expect("blob paths always contain their two-character digest directory");
        if let Err(error) = create_private_directory(parent) {
            let _ = fs::remove_file(&temporary);

            return Err(error);
        }

        match fs::rename(&temporary, &destination) {
            Ok(()) => {
                sync_directory(parent)?;
                sync_directory(&self.blobs)?;
            }
            // Retain a completed blob if another process published the same content first.
            Err(_source) if destination.is_file() => {
                let result = verify_file_digest(&destination, expected_digest);
                fs::remove_file(&temporary)
                    .map_err(|source| Error::filesystem("remove", &temporary, source))?;

                result?;
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);

                return Err(Error::filesystem("publish", &destination, error));
            }
        }

        Ok(digest)
    }

    fn verify_capture_blobs(&self, capture_id: &CaptureId) -> Result<()> {
        let mut statement = self.connection.prepare(
            "SELECT bitcode_blob FROM modules WHERE capture_id = ?1
             UNION
             SELECT text_blob FROM modules WHERE capture_id = ?1
             UNION
             SELECT blob FROM sources WHERE capture_id = ?1",
        )?;
        let digests = statement
            .query_map([capture_id.as_str()], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for digest in digests {
            let (path, expected) = self.verified_blob_path(&digest)?;
            verify_file_digest(&path, expected)?;
        }

        Ok(())
    }

    fn referenced_blob_digests(&self) -> Result<HashSet<String>> {
        let mut statement = self.connection.prepare(
            "SELECT bitcode_blob FROM modules
             UNION SELECT text_blob FROM modules
             UNION SELECT blob FROM sources",
        )?;
        let digests = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<HashSet<_>, _>>()?;

        Ok(digests)
    }

    fn blob_entries(&self) -> Result<Vec<BlobEntry>> {
        let mut blobs = Vec::new();
        let prefixes = fs::read_dir(&self.blobs)
            .map_err(|source| Error::filesystem("read", &self.blobs, source))?;

        for prefix in prefixes {
            let prefix = prefix.map_err(|source| Error::filesystem("read", &self.blobs, source))?;
            if !prefix
                .file_type()
                .map_err(|source| Error::filesystem("read metadata for", prefix.path(), source))?
                .is_dir()
            {
                continue;
            }
            let entries = fs::read_dir(prefix.path())
                .map_err(|source| Error::filesystem("read", prefix.path(), source))?;
            for entry in entries {
                let entry =
                    entry.map_err(|source| Error::filesystem("read", prefix.path(), source))?;
                let path = entry.path();
                let metadata = entry
                    .metadata()
                    .map_err(|source| Error::filesystem("read metadata for", &path, source))?;
                if !metadata.is_file() {
                    continue;
                }
                let digest = entry.file_name().to_string_lossy().into_owned();
                if digest.parse::<blake3::Hash>().is_err() {
                    continue;
                }
                blobs.push(BlobEntry {
                    path,
                    digest,
                    bytes: metadata.len(),
                });
            }
        }

        Ok(blobs)
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        let prefix = digest.get(..2).unwrap_or("00");
        self.blobs.join(prefix).join(digest)
    }

    fn capture_summary(&self, capture_id: &CaptureId, reused: bool) -> Result<CaptureSummary> {
        self.connection
            .query_row(
                "SELECT id, created_at_ms, rustc_release, llvm_version, target, profile,
                        (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id),
                        (SELECT COUNT(*) FROM modules WHERE capture_id = captures.id)
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
            InstanceMatch::Exact => {
                "definitions.path = ?2 OR display_name = ?2 OR \
                 definitions.crate_name || '::' || definitions.path = ?2"
            }
            InstanceMatch::Substring => {
                "instr(definitions.path, ?2) > 0 OR instr(display_name, ?2) > 0 OR \
                 instr(definitions.crate_name || '::' || definitions.path, ?2) > 0"
            }
        };
        let sql = format!(
            "{} WHERE instances.capture_id = ?1 AND ({predicate}) ORDER BY display_name",
            instance_select()
        );
        let mut statement = self.connection.prepare(&sql)?;
        let instances = statement
            .query_map(params![capture_id.as_str(), query], instance_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(instances)
    }

    fn read_blob(&self, digest: &str) -> Result<Vec<u8>> {
        let (path, expected) = self.verified_blob_path(digest)?;
        let mut file =
            File::open(&path).map_err(|source| Error::filesystem("open", &path, source))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| Error::filesystem("read", &path, source))?;

        verify_digest(&path, expected, blake3::hash(&bytes))?;

        Ok(bytes)
    }

    fn read_blob_range(&self, digest: &str, start: i64, end: i64) -> Result<String> {
        let (path, _) = self.verified_blob_path(digest)?;
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
        let length_u64 = end.checked_sub(start).ok_or_else(|| Error::InvalidRange {
            path: path.clone(),
            start,
            end,
        })?;
        let length = usize::try_from(length_u64).map_err(|_| Error::InvalidRange {
            path: path.clone(),
            start,
            end,
        })?;
        let mut file =
            File::open(&path).map_err(|source| Error::filesystem("open", &path, source))?;
        let file_length = file
            .metadata()
            .map_err(|source| Error::filesystem("read metadata for", &path, source))?
            .len();
        if end > file_length {
            return Err(Error::InvalidRange { path, start, end });
        }
        file.seek(SeekFrom::Start(start))
            .map_err(|source| Error::filesystem("seek", &path, source))?;
        let mut bytes = Vec::with_capacity(length);
        file.take(length_u64)
            .read_to_end(&mut bytes)
            .map_err(|source| Error::filesystem("read", &path, source))?;
        if bytes.len() != length {
            return Err(Error::InvalidRange { path, start, end });
        }

        String::from_utf8(bytes).map_err(|source| {
            Error::filesystem(
                "decode UTF-8 from",
                &path,
                io::Error::new(io::ErrorKind::InvalidData, source),
            )
        })
    }

    fn verified_blob_path(&self, digest: &str) -> Result<(PathBuf, blake3::Hash)> {
        let expected = digest.parse::<blake3::Hash>().map_err(|source| {
            Error::filesystem(
                "verify blob digest in",
                &self.blobs,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("blob digest must be a BLAKE3 hash, got {digest}: {source}"),
                ),
            )
        })?;

        Ok((self.blob_path(digest), expected))
    }
}

fn copy_and_hash_file(source: &Path, destination: &Path) -> Result<blake3::Hash> {
    let mut source_file =
        File::open(source).map_err(|error| Error::filesystem("open", source, error))?;
    let mut destination_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| Error::filesystem("create", destination, error))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let length = source_file
            .read(&mut buffer)
            .map_err(|error| Error::filesystem("read", source, error))?;

        if length == 0 {
            break;
        }

        destination_file
            .write_all(&buffer[..length])
            .map_err(|error| Error::filesystem("write", destination, error))?;
        hasher.update(&buffer[..length]);
    }

    destination_file
        .sync_all()
        .map_err(|error| Error::filesystem("sync", destination, error))?;

    Ok(hasher.finalize())
}

fn verify_file_digest(path: &Path, expected: blake3::Hash) -> Result<()> {
    let mut file = File::open(path).map_err(|source| Error::filesystem("open", path, source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|source| Error::filesystem("read", path, source))?;

        if length == 0 {
            break;
        }

        hasher.update(&buffer[..length]);
    }

    verify_digest(path, expected, hasher.finalize())
}

fn verify_digest(path: &Path, expected: blake3::Hash, actual: blake3::Hash) -> Result<()> {
    if actual == expected {
        return Ok(());
    }

    Err(Error::filesystem(
        "verify",
        path,
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("blob digest must be {expected}, got {actual}"),
        ),
    ))
}

fn sync_directory(path: &Path) -> Result<()> {
    let directory = File::open(path).map_err(|source| Error::filesystem("open", path, source))?;
    directory
        .sync_all()
        .map_err(|source| Error::filesystem("sync", path, source))
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

fn source_path_matches(stored: &str, compiler: &str) -> bool {
    if stored == compiler {
        return true;
    }

    let compiler = Path::new(compiler);
    compiler.is_relative() && Path::new(stored).ends_with(compiler)
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
    provenance: cargo_ir::ArtifactProvenance,
    bitcode_blob: String,
    text_blob: String,
    bodies: Vec<cargo_ir::BodyRange>,
    declarations: Vec<cargo_ir::LlvmDeclaration>,
    aliases: Vec<cargo_ir::LlvmAlias>,
}

struct PublishedCapture<'a> {
    capture_id: &'a CaptureId,
    request_key: &'a str,
    request_json: &'a str,
    invocation_json: &'a str,
    spec: &'a BuildSpec,
    toolchain: &'a Toolchain,
    target: &'a str,
    created_at_ms: u64,
}

struct PublishedSource {
    path: String,
    blob: String,
}

#[derive(Clone)]
struct IndexedBody {
    body_id: String,
}

struct StoredBody {
    module: String,
    symbol: String,
    text_blob: String,
    start: i64,
    end: i64,
}

struct BlobEntry {
    path: PathBuf,
    digest: String,
    bytes: u64,
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
             profile TEXT NOT NULL,
             invocation_json TEXT NOT NULL
         );
         CREATE TABLE modules(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             name TEXT NOT NULL,
             stage TEXT,
             compiler_stage TEXT NOT NULL,
             codegen_unit TEXT,
             lto TEXT NOT NULL,
             capture_method TEXT NOT NULL,
             bitcode_blob TEXT NOT NULL,
             text_blob TEXT NOT NULL
         );
         CREATE VIEW selected_modules AS
         SELECT module.* FROM modules AS module
         WHERE module.stage != 'llvm-optimized'
            OR module.compiler_stage = 'thin-lto-after-pm'
            OR NOT EXISTS (
                SELECT 1 FROM modules AS final_module
                WHERE final_module.capture_id = module.capture_id
                  AND final_module.codegen_unit IS module.codegen_unit
                  AND final_module.compiler_stage = 'thin-lto-after-pm'
            );
         CREATE TABLE definitions(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             crate_name TEXT NOT NULL,
             path TEXT NOT NULL,
             source_path TEXT,
             source_byte_start INTEGER,
             source_byte_end INTEGER,
             source_line_start INTEGER,
             source_column_start INTEGER,
             source_line_end INTEGER,
             source_column_end INTEGER
         );
         CREATE INDEX definitions_path ON definitions(capture_id, path);
         CREATE TABLE instances(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             definition_id TEXT NOT NULL REFERENCES definitions(id) ON DELETE CASCADE,
             display_name TEXT NOT NULL,
             compiler_symbol TEXT NOT NULL
         );
         CREATE INDEX instances_display_name ON instances(capture_id, display_name);
         CREATE TABLE bodies(
             id TEXT PRIMARY KEY,
             module_id TEXT NOT NULL REFERENCES modules(id) ON DELETE CASCADE,
             symbol TEXT NOT NULL,
             start INTEGER NOT NULL,
             end INTEGER NOT NULL
         );
         CREATE INDEX bodies_symbol ON bodies(module_id, symbol);
         CREATE TABLE instance_bodies(
             instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
             body_id TEXT NOT NULL REFERENCES bodies(id) ON DELETE CASCADE,
             PRIMARY KEY(instance_id, body_id)
         );
         CREATE INDEX instance_bodies_body ON instance_bodies(body_id);
         CREATE TABLE declarations(
             id TEXT PRIMARY KEY,
             module_id TEXT NOT NULL REFERENCES modules(id) ON DELETE CASCADE,
             symbol TEXT NOT NULL,
             start INTEGER NOT NULL,
             end INTEGER NOT NULL
         );
         CREATE INDEX declarations_symbol ON declarations(module_id, symbol);
         CREATE TABLE aliases(
             id TEXT PRIMARY KEY,
             module_id TEXT NOT NULL REFERENCES modules(id) ON DELETE CASCADE,
             symbol TEXT NOT NULL,
             target_kind TEXT NOT NULL,
             target_symbol TEXT,
             start INTEGER NOT NULL,
             end INTEGER NOT NULL
         );
         CREATE INDEX aliases_symbol ON aliases(module_id, symbol);
         CREATE TABLE placements(
             instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
             codegen_unit TEXT NOT NULL,
             linkage TEXT NOT NULL,
             visibility TEXT NOT NULL,
             local_copy INTEGER NOT NULL,
             size_estimate INTEGER,
             PRIMARY KEY(instance_id, codegen_unit)
         );
         CREATE TABLE sources(
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             path TEXT NOT NULL,
             blob TEXT NOT NULL,
             PRIMARY KEY(capture_id, path)
         );
         CREATE TABLE capture_cache(
             request_key TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE
         );",
    )?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    transaction.commit()?;

    Ok(())
}

fn insert_capture(transaction: &Transaction<'_>, capture: PublishedCapture<'_>) -> Result<()> {
    let created_at_ms = sqlite_integer("capture creation time", capture.created_at_ms)?;

    transaction.execute(
        "INSERT INTO captures(
             id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
             llvm_version, target, profile, invocation_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            capture.capture_id.as_str(),
            created_at_ms,
            capture.request_key,
            capture.request_json,
            capture.toolchain.release,
            capture.toolchain.commit_hash,
            capture.toolchain.llvm_version,
            capture.target,
            capture_profile_name(capture.spec.capture_profile),
            capture.invocation_json,
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
    let mut direct_aliases = Vec::new();

    for module in modules {
        let module_id = format!("mod_{}", uuid::Uuid::now_v7().simple());
        transaction.execute(
            "INSERT INTO modules(
                 id, capture_id, name, stage, compiler_stage, codegen_unit, lto,
                 capture_method, bitcode_blob, text_blob
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                module_id,
                capture_id.as_str(),
                module.name,
                module.provenance.stage.map(LlvmStage::as_str),
                module.provenance.compiler_stage,
                module.provenance.codegen_unit,
                lto_name(module.provenance.lto),
                capture_method_name(module.provenance.capture_method),
                module.bitcode_blob,
                module.text_blob,
            ],
        )?;

        for body in &module.bodies {
            let body_id = format!("body_{}", uuid::Uuid::now_v7().simple());
            let start = sqlite_integer("LLVM body start offset", body.start)?;
            let end = sqlite_integer("LLVM body end offset", body.end)?;
            transaction.execute(
                "INSERT INTO bodies(id, module_id, symbol, start, end)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![body_id, module_id, body.raw_symbol, start, end],
            )?;
            body_index
                .entry(body.raw_symbol.clone())
                .or_default()
                .push(IndexedBody { body_id });
        }

        for declaration in &module.declarations {
            transaction.execute(
                "INSERT INTO declarations(id, module_id, symbol, start, end)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    format!("decl_{}", uuid::Uuid::now_v7().simple()),
                    module_id,
                    declaration.raw_symbol,
                    sqlite_integer("LLVM declaration start offset", declaration.start)?,
                    sqlite_integer("LLVM declaration end offset", declaration.end)?,
                ],
            )?;
        }

        for alias in &module.aliases {
            let (target_kind, target_symbol) = match &alias.target {
                cargo_ir::AliasTarget::Symbol { raw_symbol } => {
                    direct_aliases.push((alias.raw_symbol.clone(), raw_symbol.clone()));
                    ("symbol", Some(raw_symbol.as_str()))
                }
                cargo_ir::AliasTarget::Expression => ("expression", None),
            };
            transaction.execute(
                "INSERT INTO aliases(
                     id, module_id, symbol, target_kind, target_symbol, start, end
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    format!("alias_{}", uuid::Uuid::now_v7().simple()),
                    module_id,
                    alias.raw_symbol,
                    target_kind,
                    target_symbol,
                    sqlite_integer("LLVM alias start offset", alias.start)?,
                    sqlite_integer("LLVM alias end offset", alias.end)?,
                ],
            )?;
        }
    }

    for (alias, target) in direct_aliases {
        if let Some(bodies) = body_index.get(&target).cloned() {
            body_index.entry(alias).or_default().extend(bodies);
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
    let mut definitions: HashMap<String, String> = HashMap::new();

    for instance in instances {
        let instance_id = InstanceId::new();
        let bodies = bodies_for_instance(instance, body_index);
        let definition_key = serde_json::to_string(&instance.origin)?;
        let definition_id = if let Some(definition_id) = definitions.get(&definition_key) {
            definition_id.clone()
        } else {
            let definition_id = insert_definition(transaction, capture_id, &instance.origin)?;
            definitions.insert(definition_key, definition_id.clone());
            definition_id
        };
        transaction.execute(
            "INSERT INTO instances(
                 id, capture_id, definition_id, display_name, compiler_symbol
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                instance_id.as_str(),
                capture_id.as_str(),
                definition_id,
                instance.display_name,
                instance.raw_symbol,
            ],
        )?;

        for body in bodies {
            transaction.execute(
                "INSERT INTO instance_bodies(instance_id, body_id) VALUES (?1, ?2)",
                params![instance_id.as_str(), body.body_id],
            )?;
        }

        for placement in &instance.placements {
            transaction.execute(
                "INSERT INTO placements(
                     instance_id, codegen_unit, linkage, visibility, local_copy, size_estimate
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    instance_id.as_str(),
                    placement.codegen_unit,
                    placement.linkage,
                    placement.visibility,
                    placement.local_copy,
                    sqlite_usize("instance size estimate", placement.size_estimate)?,
                ],
            )?;
        }
    }

    Ok(())
}

fn insert_definition(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    origin: &cargo_ir::DefinitionOrigin,
) -> Result<String> {
    let definition_id = format!("def_{}", uuid::Uuid::now_v7().simple());
    let source = origin.source.as_ref();
    transaction.execute(
        "INSERT INTO definitions(
             id, capture_id, crate_name, path, source_path, source_byte_start, source_byte_end,
             source_line_start, source_column_start, source_line_end, source_column_end
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            definition_id,
            capture_id.as_str(),
            origin.crate_name,
            origin.definition_path,
            source.map(|source| source.file_name.as_str()),
            optional_sqlite_integer("source byte start", source.map(|source| source.byte_start))?,
            optional_sqlite_integer("source byte end", source.map(|source| source.byte_end))?,
            optional_sqlite_usize("source line start", source.map(|source| source.line_start))?,
            optional_sqlite_usize(
                "source column start",
                source.map(|source| source.column_start)
            )?,
            optional_sqlite_usize("source line end", source.map(|source| source.line_end))?,
            optional_sqlite_usize("source column end", source.map(|source| source.column_end))?,
        ],
    )?;

    Ok(definition_id)
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
    let profile = capture_profile_from_name(row.get::<_, String>(5)?.as_str(), 5)?;

    Ok(CaptureSummary {
        id: row.get(0)?,
        created_at_ms: integer_from_row(row, 1)?,
        reused,
        rustc_release: row.get(2)?,
        llvm_version: row.get(3)?,
        target: row.get(4)?,
        capture_profile: profile,
        instance_count: integer_from_row(row, 6)?,
        module_count: integer_from_row(row, 7)?,
    })
}

fn artifact_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactSummary> {
    let stage = row
        .get::<_, Option<String>>(1)?
        .map(|stage| llvm_stage_from_name(&stage, 1))
        .transpose()?;

    Ok(ArtifactSummary {
        name: row.get(0)?,
        stage,
        compiler_stage: row.get(2)?,
        codegen_unit: row.get(3)?,
        lto: row.get(4)?,
        capture_method: row.get(5)?,
        definitions: integer_from_row(row, 6)?,
        declarations: integer_from_row(row, 7)?,
        aliases: integer_from_row(row, 8)?,
    })
}

fn command_view(command: cargo_ir::CommandInvocation, store_root: &Path) -> CommandView {
    CommandView {
        program: sanitize_store_path(command.program, store_root),
        arguments: command
            .arguments
            .into_iter()
            .map(|argument| sanitize_store_path(argument, store_root))
            .collect(),
    }
}

fn sanitize_store_path(value: String, store_root: &Path) -> String {
    value.replace(store_root.to_string_lossy().as_ref(), "<optic-store>")
}

fn capture_profile_from_name(value: &str, index: usize) -> rusqlite::Result<CaptureProfile> {
    match value {
        "faithful" => Ok(CaptureProfile::Faithful),
        "enriched" => Ok(CaptureProfile::Enriched),
        "experiment" => Ok(CaptureProfile::Experiment),
        _ => Err(invalid_stored_text(index, "capture profile", value)),
    }
}

fn llvm_stage_from_name(value: &str, index: usize) -> rusqlite::Result<LlvmStage> {
    match value {
        "llvm-pre-optimization" => Ok(LlvmStage::PreOptimization),
        "llvm-optimized" => Ok(LlvmStage::Optimized),
        _ => Err(invalid_stored_text(index, "LLVM stage", value)),
    }
}

fn invalid_stored_text(index: usize, name: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("stored {name} is invalid, got {value}"),
        )
        .into(),
    )
}

fn instance_select() -> &'static str {
    "SELECT instances.id, definitions.crate_name, definitions.path, instances.display_name,
            definitions.source_path, definitions.source_byte_start, definitions.source_byte_end,
            definitions.source_line_start, definitions.source_column_start,
            definitions.source_line_end, definitions.source_column_end,
            (SELECT COUNT(*) FROM instance_bodies
             JOIN bodies ON bodies.id = instance_bodies.body_id
             JOIN selected_modules AS modules ON modules.id = bodies.module_id
             WHERE instance_bodies.instance_id = instances.id
               AND modules.stage = 'llvm-optimized'),
            (SELECT COUNT(*) FROM declarations
             JOIN selected_modules AS modules ON modules.id = declarations.module_id
             WHERE modules.capture_id = instances.capture_id
               AND declarations.symbol = instances.compiler_symbol
               AND modules.stage = 'llvm-optimized'),
            (SELECT COUNT(*) FROM aliases
             JOIN selected_modules AS modules ON modules.id = aliases.module_id
             WHERE modules.capture_id = instances.capture_id
               AND aliases.symbol = instances.compiler_symbol
               AND modules.stage = 'llvm-optimized'),
            (SELECT COUNT(*) FROM instance_bodies
             JOIN bodies ON bodies.id = instance_bodies.body_id
             JOIN selected_modules AS modules ON modules.id = bodies.module_id
             WHERE instance_bodies.instance_id = instances.id
               AND modules.stage = 'llvm-pre-optimization'),
            (SELECT COUNT(*) FROM declarations
             JOIN selected_modules AS modules ON modules.id = declarations.module_id
             WHERE modules.capture_id = instances.capture_id
               AND declarations.symbol = instances.compiler_symbol
               AND modules.stage = 'llvm-pre-optimization'),
            (SELECT COUNT(*) FROM aliases
             JOIN selected_modules AS modules ON modules.id = aliases.module_id
             WHERE modules.capture_id = instances.capture_id
               AND aliases.symbol = instances.compiler_symbol
               AND modules.stage = 'llvm-pre-optimization')
     FROM instances JOIN definitions ON definitions.id = instances.definition_id"
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
    let source_path = row.get::<_, Option<String>>(4)?;
    let source = match source_path {
        Some(path) => Some(SourceLocation {
            path,
            byte_start: integer_from_row(row, 5)?,
            byte_end: integer_from_row(row, 6)?,
            line_start: integer_from_row(row, 7)?,
            column_start: integer_from_row(row, 8)?,
            line_end: integer_from_row(row, 9)?,
            column_end: integer_from_row(row, 10)?,
        }),
        None => None,
    };

    Ok(InstanceSummary {
        id: row.get(0)?,
        crate_name: row.get(1)?,
        definition: row.get(2)?,
        display_name: row.get(3)?,
        source,
        availability: vec![
            OutputAvailability {
                output: CompilerOutput::Llvm,
                definitions: integer_from_row(row, 11)?,
                declarations: integer_from_row(row, 12)?,
                aliases: integer_from_row(row, 13)?,
            },
            OutputAvailability {
                output: CompilerOutput::LlvmPreOpt,
                definitions: integer_from_row(row, 14)?,
                declarations: integer_from_row(row, 15)?,
                aliases: integer_from_row(row, 16)?,
            },
        ],
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

fn sqlite_usize(name: &'static str, value: usize) -> Result<i64> {
    let value = u64::try_from(value).map_err(|_| Error::IntegerOutOfRange {
        name,
        maximum: u64::MAX.into(),
        actual: value as u128,
    })?;

    sqlite_integer(name, value)
}

fn optional_sqlite_integer(name: &'static str, value: Option<u64>) -> Result<Option<i64>> {
    value.map(|value| sqlite_integer(name, value)).transpose()
}

fn optional_sqlite_usize(name: &'static str, value: Option<usize>) -> Result<Option<i64>> {
    value.map(|value| sqlite_usize(name, value)).transpose()
}

const fn capture_profile_name(profile: crate::CaptureProfile) -> &'static str {
    match profile {
        crate::CaptureProfile::Faithful => "faithful",
        crate::CaptureProfile::Enriched => "enriched",
        crate::CaptureProfile::Experiment => "experiment",
    }
}

const fn lto_name(lto: cargo_ir::LtoScope) -> &'static str {
    match lto {
        cargo_ir::LtoScope::None => "none",
        cargo_ir::LtoScope::Thin => "thin",
        cargo_ir::LtoScope::Unknown => "unknown",
    }
}

const fn capture_method_name(method: cargo_ir::CaptureMethod) -> &'static str {
    match method {
        cargo_ir::CaptureMethod::SavedTemporary => "saved-temporary",
    }
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
    use std::fs;
    use std::io;

    use cargo_ir::{CompilerInstance, DefinitionOrigin};
    use rusqlite::Connection;

    use super::{IndexedBody, STORE_VERSION, Store, bodies_for_instance, unique_prefix_length};
    use crate::{CaptureId, Error, InstanceId};

    #[test]
    fn associates_an_instance_only_with_its_exact_compiler_symbol() {
        let instance = CompilerInstance {
            origin: DefinitionOrigin {
                crate_name: "mask_iteration".to_owned(),
                definition_path: "mask_iteration::for_each_set_index".to_owned(),
                source: None,
            },
            display_name: "mask_iteration::for_each_set_index".to_owned(),
            raw_symbol: "_Rmask_iteration".to_owned(),
            placements: Vec::new(),
        };
        let mut body_index = HashMap::new();
        body_index.insert(
            "_Rmask_iteration".to_owned(),
            vec![IndexedBody {
                body_id: "body".to_owned(),
            }],
        );

        let bodies = bodies_for_instance(&instance, &body_index);

        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].body_id, "body");
    }

    #[test]
    fn does_not_associate_an_llvm_clone_by_its_display_name() {
        let instance = CompilerInstance {
            origin: DefinitionOrigin {
                crate_name: "mask_iteration".to_owned(),
                definition_path: "mask_iteration::make".to_owned(),
                source: None,
            },
            display_name: "mask_iteration::make".to_owned(),
            raw_symbol: "_Rmake".to_owned(),
            placements: Vec::new(),
        };
        let body_index = HashMap::from([(
            "_Rmake.llvm.123".to_owned(),
            vec![IndexedBody {
                body_id: "clone".to_owned(),
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
    fn matches_only_exact_or_relative_captured_source_paths() {
        assert!(super::source_path_matches(
            "/workspace/kernel/src/lib.rs",
            "/workspace/kernel/src/lib.rs"
        ));
        assert!(super::source_path_matches(
            "/workspace/kernel/src/lib.rs",
            "kernel/src/lib.rs"
        ));
        assert!(!super::source_path_matches(
            "/workspace/kernel/src/lib.rs",
            "/other/kernel/src/lib.rs"
        ));
    }

    #[test]
    fn selects_the_final_thin_lto_artifact_as_optimized_output() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        store
            .connection
            .execute_batch(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_release, rustc_commit,
                     llvm_version, target, profile, invocation_json
                 ) VALUES ('cap_00000000000000000000000000000000', 0, 'key', '{}', '', '', '',
                           '', 'faithful', '{}');
                 INSERT INTO modules(
                     id, capture_id, name, stage, compiler_stage, codegen_unit, lto,
                     capture_method, bitcode_blob, text_blob
                 ) VALUES
                     ('mod_rcgu', 'cap_00000000000000000000000000000000', 'rcgu',
                      'llvm-optimized', 'rcgu', 'crate.cgu.0', 'none', 'saved-temporary', 'a', 'b'),
                     ('mod_thin', 'cap_00000000000000000000000000000000', 'thin',
                      'llvm-optimized', 'thin-lto-after-pm', 'crate.cgu.0', 'thin',
                      'saved-temporary', 'c', 'd');",
            )
            .expect("the test can insert ThinLTO artifacts");
        let selected = store
            .connection
            .query_row(
                "SELECT name FROM selected_modules WHERE stage = 'llvm-optimized'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("the selected optimized artifact is readable");

        assert_eq!(selected, "thin");
    }

    #[test]
    fn publishes_and_reuses_a_verified_blob() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        let contents = b"define void @kernel() {}\n";
        fs::write(&source, contents).expect("the test can write source evidence");

        let digest = store
            .publish_blob(&source)
            .expect("the store can publish the source evidence");
        let reused_digest = store
            .publish_blob(&source)
            .expect("the store can verify and reuse the published evidence");
        let destination = store.blob_path(&digest);

        assert_eq!(digest, blake3::hash(contents).to_hex().as_str());
        assert_eq!(reused_digest, digest);
        assert_eq!(
            fs::read(destination).expect("the test can read the published evidence"),
            contents
        );
        assert!(temporary_blob_paths(&store).is_empty());
    }

    #[test]
    fn rejects_a_corrupt_existing_blob_before_reuse() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        fs::write(&source, b"original").expect("the test can write source evidence");
        let digest = store
            .publish_blob(&source)
            .expect("the store can publish the source evidence");
        fs::write(store.blob_path(&digest), b"corrupt")
            .expect("the test can corrupt the published evidence");

        assert_invalid_data(store.publish_blob(&source));
        assert!(temporary_blob_paths(&store).is_empty());
    }

    #[test]
    fn rejects_corrupt_full_blob_reads_and_invalid_ranges() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        fs::write(&source, b"original").expect("the test can write source evidence");
        let digest = store
            .publish_blob(&source)
            .expect("the store can publish the source evidence");
        fs::write(store.blob_path(&digest), b"corrupt")
            .expect("the test can corrupt the published evidence");

        assert_invalid_data(store.read_blob(&digest));
        assert!(matches!(
            store.read_blob_range(&digest, 0, 8),
            Err(Error::InvalidRange { .. })
        ));
    }

    #[test]
    fn rejects_invalid_utf8_in_a_verified_blob_range() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        fs::write(&source, [0xff]).expect("the test can write non-UTF-8 evidence");
        let digest = store
            .publish_blob(&source)
            .expect("the store can publish the source evidence");

        assert_invalid_data(store.read_blob_range(&digest, 0, 1));
    }

    #[test]
    fn rejects_a_schema_four_store() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let catalog = temporary.path().join("catalog.sqlite");
        let connection = Connection::open(&catalog).expect("the test can open the catalog");
        connection
            .pragma_update(None, "user_version", 4)
            .expect("the test can create a schema-four catalog");
        drop(connection);

        let optic = temporary.path().join(".optic");
        fs::create_dir(&optic).expect("the test can create the store directory");
        fs::rename(catalog, optic.join("catalog.sqlite"))
            .expect("the test can install the schema-four catalog");

        assert!(matches!(
            Store::open(temporary.path()),
            Err(Error::StoreVersion {
                expected: STORE_VERSION,
                actual: 4,
            })
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
                     llvm_version, target, profile, invocation_json
                 ) VALUES
                     ('cap_00000000000000000000000000000000', 0, 'a', '{}', '', '', '', '', '', '{}'),
                     ('cap_0fffffffffffffffffffffffffffffff', 0, 'b', '{}', '', '', '', '', '', '{}');
                 INSERT INTO definitions(id, capture_id, crate_name, path) VALUES
                     ('def_00000000000000000000000000000000',
                      'cap_00000000000000000000000000000000', '', ''),
                     ('def_0fffffffffffffffffffffffffffffff',
                      'cap_0fffffffffffffffffffffffffffffff', '', '');
                 INSERT INTO instances(
                     id, capture_id, definition_id, display_name, compiler_symbol
                 ) VALUES
                     ('ins_00000000000000000000000000000000',
                      'cap_00000000000000000000000000000000',
                      'def_00000000000000000000000000000000', '', ''),
                     ('ins_0fffffffffffffffffffffffffffffff',
                      'cap_0fffffffffffffffffffffffffffffff',
                      'def_0fffffffffffffffffffffffffffffff', '', '');",
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
                     llvm_version, target, profile, invocation_json
                 ) VALUES ('invalid', 0, 'request', '{}', '', '', '', '', '', '{}')",
                [],
            )
            .expect("the test can insert an invalid stored identifier");

        assert!(matches!(store.captures(), Err(Error::Database(_))));
    }

    #[track_caller]
    fn assert_invalid_data<T>(result: crate::Result<T>) {
        match result {
            Err(Error::Filesystem { source, .. }) => {
                assert_eq!(source.kind(), io::ErrorKind::InvalidData);
            }
            Err(error) => panic!("the operation must report invalid blob data, got {error}"),
            Ok(_) => panic!("the operation must reject invalid blob data"),
        }
    }

    fn temporary_blob_paths(store: &Store) -> Vec<std::path::PathBuf> {
        fs::read_dir(&store.blobs)
            .expect("the test can inspect the blob directory")
            .map(|entry| entry.expect("the blob directory entry is readable").path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".tmp"))
            })
            .collect()
    }
}
