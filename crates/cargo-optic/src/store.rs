//! Persists immutable captures and serves byte-range queries.
//!
//! [`Store`] owns `SQLite` schema details and content-addressed blobs. Callers see opaque IDs and
//! typed views. A capture becomes visible only after every blob is durable and one catalog
//! transaction commits.

#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cargo_ir::{CompiledCapture, EvidenceEvent, LlvmStage, Toolchain};
use fs2::FileExt;
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use walkdir::WalkDir;

use crate::pending::PendingCapture;
use crate::source::{SourceBaseline, StoredSource};
use crate::{
    ArtifactSummary, BodyView, BuildSpec, CaptureDetails, CaptureDisposition, CaptureId,
    CaptureMetadata, CaptureProfile, CaptureSummary, CommandView, CompilerOutput,
    CompilerProvenance, EnvironmentView, Error, FindMatchKind, FindOptions, FindResult, GcSummary,
    InstanceId, InstanceSummary, OutputAvailability, PendingId, PendingRemoveSummary,
    PendingSummary, RemarkCaptureSummary, RemarkEvidenceState, RemarkFileSummary, RemarkKindFilter,
    RemarkOptions, RemarkShowView, RemarkView, RemoveSummary, Result, ShowEvent, ShowView,
    SourceLocation, StoreStatus, TEXT_CHUNK_BYTES, VerifySummary,
};

const STORE_VERSION: u32 = 10;
// Level 3 reduced the 319,120,141-byte prototype corpus to 36,326,789 bytes while encoding above
// 2 GB/s. Higher levels would spend more capture time for a secondary prototype concern.
const BLOB_COMPRESSION_LEVEL: i32 = 3;
// This limits an over-budget staging catalog to 100,000 new records without walking the store for
// every remark in a multi-million-record capture.
const STORAGE_BUDGET_EVENT_INTERVAL: usize = 100_000;

#[derive(Clone, Debug)]
pub(crate) struct AnalysisKey(String);

impl AnalysisKey {
    pub(crate) fn new() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        let valid = value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(Error::InvalidPendingEvidence {
                path: PathBuf::from("pending.json"),
                message: format!(
                    "analysis key must contain 32 lowercase hexadecimal characters, got {value}"
                ),
            });
        }

        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl rusqlite::types::FromSql for AnalysisKey {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let value = value.as_str()?;
        let valid = value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(rusqlite::types::FromSqlError::Other(Box::new(
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("stored analysis key is invalid, got {value}"),
                ),
            )));
        }

        Ok(Self(value.to_owned()))
    }
}

pub(crate) struct CachedCapture {
    pub(crate) analysis_key: AnalysisKey,
    pub(crate) summary: CaptureSummary,
}

pub(crate) struct CaptureCacheKey<'a> {
    request: &'a str,
    analysis: &'a AnalysisKey,
}

impl<'a> CaptureCacheKey<'a> {
    pub(crate) fn new(request: &'a str, analysis: &'a AnalysisKey) -> Self {
        Self { request, analysis }
    }
}

pub(crate) struct CapturePublication<'a> {
    pub(crate) capture_id: &'a CaptureId,

    pub(crate) cache_key: CaptureCacheKey<'a>,

    pub(crate) spec: &'a BuildSpec,

    pub(crate) sources: &'a SourceBaseline,

    pub(crate) target: &'a str,

    pub(crate) maximum_store_bytes: Option<u64>,
}

pub(crate) struct Store {
    optic: PathBuf,

    root: PathBuf,

    locks: PathBuf,

    blobs: PathBuf,

    pending: PathBuf,

    connection: Connection,

    access: StoreAccess,
}

enum StoreAccess {
    ReadWrite,

    ReadOnly { optic_dir: PathBuf },
}

pub(crate) struct FileLock {
    /// The operating system releases the lock when this file is dropped.
    _file: File,
}

/// Prevents evidence removal while a command uses the workspace store.
pub(crate) fn lock_workspace_shared(workspace_root: &Path) -> Result<FileLock> {
    let optic = workspace_root.join(".optic");
    create_private_directory(&optic)?;
    let locks = optic.join("locks");
    create_private_directory(&locks)?;
    let path = locks.join("operation.lock");
    let file = open_lock_file(&path)?;
    FileExt::lock_shared(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

    Ok(FileLock { _file: file })
}

/// Prevents evidence removal while a command reads an existing `.optic` store.
pub(crate) fn lock_optic_shared(optic: &Path) -> Result<FileLock> {
    let path = optic.join("locks/operation.lock");
    let file = open_existing_lock_file(&path)?;
    FileExt::lock_shared(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

    Ok(FileLock { _file: file })
}

/// Waits for active commands and prevents new commands from opening the workspace store.
pub(crate) fn lock_workspace_exclusive(workspace_root: &Path) -> Result<FileLock> {
    let optic = workspace_root.join(".optic");
    create_private_directory(&optic)?;
    let locks = optic.join("locks");
    create_private_directory(&locks)?;
    let path = locks.join("operation.lock");
    let file = open_lock_file(&path)?;
    FileExt::lock_exclusive(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

    Ok(FileLock { _file: file })
}

impl Store {
    pub(crate) fn open(workspace_root: &Path) -> Result<Self> {
        let optic = workspace_root.join(".optic");
        reject_legacy_store(&optic)?;

        let root = optic.join("store");
        let blobs = root.join("blobs");
        let pending = root.join("pending");
        let work = root.join("work");
        let locks = optic.join("locks");

        for directory in [&optic, &locks, &root, &blobs, &pending, &work] {
            create_private_directory(directory)?;
        }

        for name in ["operation.lock", "writer.lock", "evidence.lock"] {
            drop(open_lock_file(&locks.join(name))?);
        }

        let _schema_lock = lock_file(&locks.join("schema.lock"))?;
        let mut connection = Connection::open(root.join("catalog.sqlite"))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        initialize_schema(&mut connection)?;

        Ok(Self {
            optic,
            root,
            locks,
            blobs,
            pending,
            connection,
            access: StoreAccess::ReadWrite,
        })
    }

    pub(crate) fn open_read_only(optic: &Path) -> Result<Self> {
        let root = optic.join("store");
        let blobs = root.join("blobs");
        let pending = root.join("pending");
        let locks = optic.join("locks");
        let schema_path = locks.join("schema.lock");
        let schema_file = open_existing_lock_file(&schema_path)?;
        FileExt::lock_shared(&schema_file)
            .map_err(|source| Error::filesystem("lock", &schema_path, source))?;
        let _schema_lock = FileLock { _file: schema_file };
        let connection = Connection::open_with_flags(
            root.join("catalog.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "query_only", true)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        validate_schema(&connection)?;

        Ok(Self {
            optic: optic.to_owned(),
            root,
            locks,
            blobs,
            pending,
            connection,
            access: StoreAccess::ReadOnly {
                optic_dir: optic.to_owned(),
            },
        })
    }

    pub(crate) fn pending_directory(&self, request_key: &str) -> Result<PathBuf> {
        validate_request_key(request_key)?;

        Ok(self.pending.join(request_key))
    }

    pub(crate) fn lock_writer(&self) -> Result<FileLock> {
        self.require_writable("write evidence")?;
        let path = self.locks.join("writer.lock");

        lock_file(&path)
    }

    pub(crate) fn lock_pending_reader(&self) -> Result<FileLock> {
        let path = self.locks.join("writer.lock");
        let file = match &self.access {
            StoreAccess::ReadWrite => open_lock_file(&path)?,
            StoreAccess::ReadOnly { .. } => open_existing_lock_file(&path)?,
        };
        FileExt::lock_shared(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

        Ok(FileLock { _file: file })
    }

    pub(crate) fn lock_evidence_reader(&self) -> Result<FileLock> {
        let path = self.locks.join("evidence.lock");
        let file = match &self.access {
            StoreAccess::ReadWrite => open_lock_file(&path)?,
            StoreAccess::ReadOnly { .. } => open_existing_lock_file(&path)?,
        };
        FileExt::lock_shared(&file).map_err(|source| Error::filesystem("lock", &path, source))?;

        Ok(FileLock { _file: file })
    }

    fn lock_evidence_writer(&self) -> Result<FileLock> {
        self.require_writable("update evidence")?;
        let path = self.locks.join("evidence.lock");

        lock_file(&path)
    }

    fn require_writable(&self, operation: &'static str) -> Result<()> {
        match &self.access {
            StoreAccess::ReadWrite => Ok(()),
            StoreAccess::ReadOnly { optic_dir } => Err(Error::ReadOnlyStore {
                operation,
                path: optic_dir.clone(),
            }),
        }
    }

    pub(crate) fn cached_capture(&self, request_key: &str) -> Result<Option<CachedCapture>> {
        let cached = self
            .connection
            .query_row(
                "SELECT capture_id, analysis_key FROM capture_cache WHERE request_key = ?1",
                [request_key],
                |row| Ok((row.get::<_, CaptureId>(0)?, row.get::<_, AnalysisKey>(1)?)),
            )
            .optional()?;

        cached
            .map(|(capture_id, analysis_key)| {
                self.verify_capture_blobs(&capture_id)?;
                Ok(CachedCapture {
                    analysis_key,
                    summary: self.capture_summary(&capture_id, CaptureDisposition::Reused)?,
                })
            })
            .transpose()
    }

    pub(crate) fn publish_stream(
        &mut self,
        compilation: CompiledCapture,
        publication: CapturePublication<'_>,
    ) -> Result<CaptureSummary> {
        let CapturePublication {
            capture_id,
            cache_key,
            spec,
            sources,
            target,
            maximum_store_bytes,
        } = publication;
        let analysis_directory = compilation.invocation.request.analysis_directory.clone();
        let staging_path = analysis_directory
            .parent()
            .expect("analysis directories are created below one pending run directory")
            .join("ingest.sqlite");
        let mut staging = StagedCapture::create(&staging_path)?;
        let mut staging_error = None;
        let metadata = cargo_ir::ingest_with_events(
            &compilation.invocation.request.clone(),
            compilation,
            |event| {
                if staging_error.is_some() {
                    return;
                }

                if let Err(error) = staging.push(self, event, maximum_store_bytes) {
                    staging_error = Some(error);
                }
            },
        )?;
        if let Some(error) = staging_error {
            return Err(error);
        }
        if spec.capture_remarks != metadata.remarks_captured {
            return Err(Error::InvalidStoredData {
                message: "remark evidence must match the capture request".to_owned(),
            });
        }

        for source in &sources.entries {
            staging.push_source(self, source, maximum_store_bytes)?;
        }

        let staged = staging.finish()?;
        self.ensure_storage_budget(maximum_store_bytes)?;
        self.commit_staged_capture(capture_id, cache_key, spec, &metadata, target, staged)?;

        self.capture_summary(capture_id, CaptureDisposition::Captured)
    }

    fn commit_staged_capture(
        &mut self,
        capture_id: &CaptureId,
        cache_key: CaptureCacheKey<'_>,
        spec: &BuildSpec,
        metadata: &cargo_ir::EvidenceMetadata,
        target: &str,
        staged: CompletedStaging,
    ) -> Result<()> {
        let staging_path = staged.path.to_string_lossy().into_owned();
        self.connection
            .execute("ATTACH DATABASE ?1 AS staged", [&staging_path])?;
        let result = (|| {
            let created_at_ms = now_ms()?;
            let request_json = serde_json::to_string(spec)?;
            let invocation_json = serde_json::to_string(&metadata.invocation)?;
            let remarks = remark_summary(
                metadata.remarks_captured,
                staged.remark_files,
                staged.remark_records,
                staged.linked_remark_records,
            );
            let transaction = self.connection.transaction()?;
            insert_capture(
                &transaction,
                PublishedCapture {
                    capture_id,
                    request_key: cache_key.request,
                    request_json: &request_json,
                    invocation_json: &invocation_json,
                    spec,
                    toolchain: &metadata.toolchain,
                    target,
                    created_at_ms,
                    remarks,
                },
            )?;
            transaction.execute(
                "INSERT INTO definitions(
                     id, capture_id, crate_name, path, source_path, source_byte_start,
                     source_byte_end, source_line_start, source_column_start, source_line_end,
                     source_column_end, source_item_start, source_item_end,
                     source_item_line_start
                 )
                 SELECT id, ?1, crate_name, path, source_path, source_byte_start,
                        source_byte_end, source_line_start, source_column_start, source_line_end,
                        source_column_end, source_item_start, source_item_end,
                        source_item_line_start
                 FROM staged.definitions",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO instances(
                     id, capture_id, definition_id, display_name, compiler_symbol
                 )
                 SELECT id, ?1, definition_id, display_name, compiler_symbol
                 FROM staged.instances",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO placements(
                     instance_id, codegen_unit, linkage, visibility, local_copy, size_estimate
                 )
                 SELECT instance_id, codegen_unit, linkage, visibility, local_copy, size_estimate
                 FROM staged.placements",
                [],
            )?;
            transaction.execute(
                "INSERT INTO modules(
                     id, capture_id, name, stage, compiler_stage, codegen_unit, lto,
                     capture_method, bitcode_blob, text_blob
                 )
                 SELECT id, ?1, name, stage, compiler_stage, codegen_unit, lto,
                        capture_method, bitcode_blob, text_blob
                 FROM staged.modules",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO bodies(id, module_id, symbol, start, end)
                 SELECT id, module_id, symbol, start, end FROM staged.bodies",
                [],
            )?;
            transaction.execute(
                "INSERT INTO declarations(id, module_id, symbol, start, end)
                 SELECT id, module_id, symbol, start, end FROM staged.declarations",
                [],
            )?;
            transaction.execute(
                "INSERT INTO aliases(
                     id, module_id, symbol, target_kind, target_symbol, start, end
                 )
                 SELECT id, module_id, symbol, target_kind, target_symbol, start, end
                 FROM staged.aliases",
                [],
            )?;
            associate_streamed_bodies(&transaction, capture_id)?;
            update_streamed_availability(&transaction, capture_id)?;
            transaction.execute(
                "INSERT INTO instance_search(
                     rowid, instance_id, capture_id, definition_path, display_name,
                     compiler_symbol
                 )
                 SELECT instances.rowid, instances.id, instances.capture_id,
                        definitions.path, instances.display_name, instances.compiler_symbol
                 FROM instances
                 JOIN definitions ON definitions.id = instances.definition_id
                 WHERE instances.capture_id = ?1",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO remark_files(id, capture_id, name, blob, record_count)
                 SELECT id, ?1, name, blob, record_count FROM staged.remark_files",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO remarks(
                     id, file_id, ordinal, kind, unknown_kind, pass_name, remark_name,
                     function_symbol, source_file, source_line, source_column, hotness,
                     arguments_json, message
                 )
                 SELECT id, file_id, ordinal, kind, unknown_kind, pass_name, remark_name,
                        function_symbol, source_file, source_line, source_column, hotness,
                        arguments_json, message
                 FROM staged.remarks",
                [],
            )?;
            transaction.execute(
                "INSERT INTO remark_instances(remark_id, instance_id)
                 SELECT remarks.id, instances.id
                 FROM remarks
                 JOIN remark_files ON remark_files.id = remarks.file_id
                 JOIN instances ON instances.compiler_symbol = remarks.function_symbol
                               AND instances.capture_id = remark_files.capture_id
                 WHERE remark_files.capture_id = ?1",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO sources(capture_id, path, blob)
                 SELECT ?1, path, blob FROM staged.sources",
                [capture_id.as_str()],
            )?;
            transaction.execute(
                "INSERT INTO capture_cache(request_key, capture_id, analysis_key)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(request_key) DO UPDATE SET
                     capture_id = excluded.capture_id,
                     analysis_key = excluded.analysis_key",
                params![
                    cache_key.request,
                    capture_id.as_str(),
                    cache_key.analysis.as_str()
                ],
            )?;
            transaction.commit()?;

            Ok(())
        })();
        let detach_result = self.connection.execute_batch("DETACH DATABASE staged");

        match (result, detach_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error.into()),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    pub(crate) fn captures(&self) -> Result<Vec<CaptureSummary>> {
        let mut captures = Vec::new();
        self.stream_captures(&mut |capture| {
            captures.push(capture);

            std::ops::ControlFlow::Continue(())
        })?;

        Ok(captures)
    }

    pub(crate) fn stream_captures(
        &self,
        on_capture: &mut impl FnMut(CaptureSummary) -> std::ops::ControlFlow<()>,
    ) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT id, created_at_ms, rustc_release, llvm_version, target, profile,
                    (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id),
                    (SELECT COUNT(*) FROM modules WHERE capture_id = captures.id),
                    remarks_captured, remark_file_count, remark_record_count,
                    remark_linked_record_count
             FROM captures ORDER BY created_at_ms DESC",
        )?;
        let mut rows = statement.query([])?;
        let mut count = 0;

        while let Some(row) = rows.next()? {
            let capture = summary_from_row(row, CaptureDisposition::Captured)?;
            if on_capture(capture).is_break() {
                return Err(Error::ConsumerStopped);
            }
            count += 1;
        }

        Ok(count)
    }

    pub(crate) fn status(&self) -> Result<StoreStatus> {
        let captures = self
            .connection
            .query_row("SELECT COUNT(*) FROM captures", [], |row| {
                integer_from_row(row, 0)
            })?;
        let blobs = self.blob_entries()?;
        let referenced = self.referenced_blob_digests()?;
        let referenced_blob_bytes = blobs
            .iter()
            .filter(|blob| referenced.contains(&blob.digest))
            .map(|blob| blob.bytes)
            .sum();
        let unreferenced_blob_bytes = blobs
            .iter()
            .filter(|blob| !referenced.contains(&blob.digest))
            .map(|blob| blob.bytes)
            .sum();

        let (pending, pending_bytes) = pending_entries(&self.pending)?;
        let retained_bytes = directory_bytes(&self.root)?;
        let (available_bytes, policy) = self.storage_policy(None)?;

        Ok(StoreStatus {
            captures,
            blobs: blobs.len(),
            blob_bytes: blobs.iter().map(|blob| blob.bytes).sum(),
            referenced_blob_bytes,
            unreferenced_blob_bytes,
            pending,
            pending_bytes,
            retained_bytes,
            available_bytes,
            maximum_bytes: policy.maximum_bytes,
            minimum_available_bytes: policy.available_space_reserve,
        })
    }

    pub(crate) fn ensure_storage_budget(&self, command_maximum_bytes: Option<u64>) -> Result<()> {
        let retained_bytes = directory_bytes(&self.root)?;
        let (available_bytes, policy) = self.storage_policy(command_maximum_bytes)?;
        if retained_bytes < policy.maximum_bytes && available_bytes > policy.available_space_reserve
        {
            return Ok(());
        }

        Err(Error::StoreBudgetExceeded {
            retained_bytes,
            maximum_bytes: policy.maximum_bytes,
            available_bytes,
            minimum_available_bytes: policy.available_space_reserve,
        })
    }

    fn storage_policy(
        &self,
        command_maximum_bytes: Option<u64>,
    ) -> Result<(u64, crate::config::StorePolicy)> {
        let filesystem_bytes = fs2::total_space(&self.root).map_err(|source| {
            Error::filesystem("read filesystem capacity for", &self.root, source)
        })?;
        let available_bytes = fs2::available_space(&self.root)
            .map_err(|source| Error::filesystem("read available space for", &self.root, source))?;
        let policy =
            crate::config::load_store_policy(&self.optic, command_maximum_bytes, filesystem_bytes)?;

        Ok((available_bytes, policy))
    }

    pub(crate) fn pending(&self) -> Result<Vec<PendingSummary>> {
        Ok(self
            .pending_entries()?
            .into_iter()
            .map(|entry| entry.summary)
            .collect())
    }

    pub(crate) fn pending_summary(&self, pending_id: &PendingId) -> Result<PendingSummary> {
        Ok(self.resolve_pending(pending_id)?.summary)
    }

    pub(crate) fn unique_pending_prefix(&self, pending_id: &PendingId) -> Result<PendingId> {
        let entries = self.pending_entries()?;
        let index = entries
            .binary_search_by(|entry| entry.summary.id.as_str().cmp(pending_id.as_str()))
            .map_err(|_| Error::UnknownPending {
                pending_id: pending_id.clone(),
            })?;
        let previous = index
            .checked_sub(1)
            .map(|index| entries[index].summary.id.as_str());
        let next = entries
            .get(index + 1)
            .map(|entry| entry.summary.id.as_str());
        let length = unique_prefix_length(pending_id.as_str(), previous, next);

        Ok(pending_id.as_str()[..length]
            .parse()
            .expect("the pending prefix comes from a validated pending-capture ID"))
    }

    pub(crate) fn remove_pending(&self, pending_id: &PendingId) -> Result<PendingRemoveSummary> {
        self.require_writable("remove pending evidence")?;
        let entry = self.resolve_pending(pending_id)?;
        fs::remove_dir_all(&entry.path)
            .map_err(|source| Error::filesystem("remove", &entry.path, source))?;
        sync_directory(&self.pending)?;

        Ok(PendingRemoveSummary {
            id: entry.summary.id,
            removed_bytes: entry.summary.retained_bytes,
        })
    }

    pub(crate) fn remove_capture(&mut self, capture_prefix: &CaptureId) -> Result<RemoveSummary> {
        let _evidence = self.lock_evidence_writer()?;
        let capture_id = self.resolve_capture(capture_prefix)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM instance_search WHERE capture_id = ?1",
            [capture_id.as_str()],
        )?;
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
        self.verify_search_index()?;
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
        let metadata = self.capture_metadata(capture_prefix)?;
        let capture_id = metadata.summary.id.clone();
        let mut artifacts = Vec::new();
        self.stream_capture_artifacts(&capture_id, &mut |artifact| {
            artifacts.push(artifact);

            ControlFlow::Continue(())
        })?;
        let mut remark_files = Vec::new();
        self.stream_capture_remark_files(&capture_id, &mut |remark_file| {
            remark_files.push(remark_file);

            ControlFlow::Continue(())
        })?;
        let CaptureMetadata {
            summary,
            request,
            compiler,
            unstable_access,
            cargo,
            rustc,
            wrapper_chain,
            environment,
            injected_rustc_arguments,
        } = metadata;

        Ok(CaptureDetails {
            summary,
            request,
            compiler,
            unstable_access,
            cargo,
            rustc,
            wrapper_chain,
            environment,
            injected_rustc_arguments,
            artifacts,
            remark_files,
        })
    }

    pub(crate) fn capture_metadata(&self, capture_prefix: &CaptureId) -> Result<CaptureMetadata> {
        let capture_id = self.resolve_capture(capture_prefix)?;
        let summary = self.capture_summary(&capture_id, CaptureDisposition::Captured)?;
        let (request_json, invocation_json, compiler) = self.connection.query_row(
            "SELECT request_json, invocation_json, rustc_path, rustc_release, rustc_commit,
                    rustc_host, llvm_version, rustc_sysroot, llvm_dis_path
             FROM captures WHERE id = ?1",
            [capture_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    CompilerProvenance {
                        rustc: PathBuf::from(row.get::<_, String>(2)?),
                        release: row.get(3)?,
                        commit_hash: row.get(4)?,
                        host: row.get(5)?,
                        llvm_version: row.get(6)?,
                        sysroot: PathBuf::from(row.get::<_, String>(7)?),
                        llvm_dis: PathBuf::from(row.get::<_, String>(8)?),
                    },
                ))
            },
        )?;
        let request = serde_json::from_str::<BuildSpec>(&request_json)?;
        let invocation = serde_json::from_str::<cargo_ir::CaptureInvocation>(&invocation_json)?;

        Ok(CaptureMetadata {
            summary,
            request,
            compiler,
            unstable_access: invocation.unstable_access,
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
        })
    }

    pub(crate) fn stream_capture_artifacts(
        &self,
        capture_id: &CaptureId,
        on_artifact: &mut impl FnMut(ArtifactSummary) -> ControlFlow<()>,
    ) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT modules.name, modules.stage, modules.compiler_stage, modules.codegen_unit,
                    modules.lto, modules.capture_method,
                    (SELECT COUNT(*) FROM bodies WHERE module_id = modules.id),
                    (SELECT COUNT(*) FROM declarations WHERE module_id = modules.id),
                    (SELECT COUNT(*) FROM aliases WHERE module_id = modules.id)
             FROM modules WHERE capture_id = ?1 ORDER BY modules.name, modules.compiler_stage",
        )?;
        let mut artifacts =
            statement.query_map([capture_id.as_str()], artifact_summary_from_row)?;
        let mut count = 0;
        for artifact in &mut artifacts {
            count += 1;
            if on_artifact(artifact?).is_break() {
                return Err(Error::ConsumerStopped);
            }
        }

        Ok(count)
    }

    pub(crate) fn stream_capture_remark_files(
        &self,
        capture_id: &CaptureId,
        on_remark_file: &mut impl FnMut(RemarkFileSummary) -> ControlFlow<()>,
    ) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT name, record_count FROM remark_files
             WHERE capture_id = ?1 ORDER BY name",
        )?;
        let mut remark_files = statement.query_map([capture_id.as_str()], |row| {
            Ok(RemarkFileSummary {
                name: row.get(0)?,
                records: integer_from_row(row, 1)?,
            })
        })?;
        let mut count = 0;
        for remark_file in &mut remark_files {
            count += 1;
            if on_remark_file(remark_file?).is_break() {
                return Err(Error::ConsumerStopped);
            }
        }

        Ok(count)
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

    pub(crate) fn find(
        &self,
        capture_prefix: &CaptureId,
        options: &FindOptions,
    ) -> Result<FindResult> {
        let capture_id = self.resolve_capture(capture_prefix)?;
        let exact = self.query_instance_ids(&capture_id, InstanceMatch::Exact, options)?;
        let (match_kind, mut instance_ids) = if exact.is_empty() {
            if options.query.chars().count() < 3 {
                return Err(Error::InvalidRequest {
                    message: format!(
                        "substring queries must contain at least 3 Unicode characters after no exact match, got {}",
                        options.query.chars().count()
                    ),
                });
            }

            (
                FindMatchKind::Substring,
                self.query_instance_ids(&capture_id, InstanceMatch::Substring, options)?,
            )
        } else {
            (FindMatchKind::Exact, exact)
        };
        let truncated = instance_ids.len() > options.limit;
        instance_ids.truncate(options.limit);
        let instances = self.hydrate_instances(&instance_ids)?;

        Ok(FindResult {
            capture_id,
            match_kind,
            truncated,
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

    pub(crate) fn prepare_show(
        &self,
        instance_prefix: &InstanceId,
        output: CompilerOutput,
    ) -> Result<PreparedShow> {
        let resolved = self.resolve_instance(instance_prefix)?;
        let instance = self.connection.query_row(
            &format!("{} WHERE instances.id = ?1", instance_select()),
            [resolved.instance_id.as_str()],
            instance_from_row,
        )?;

        Ok(PreparedShow {
            capture_id: resolved.capture_id,
            instance,
            output,
        })
    }

    pub(crate) fn stream_show_bodies(
        &self,
        show: &PreparedShow,
        on_event: &mut impl FnMut(ShowEvent) -> std::ops::ControlFlow<()>,
    ) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT modules.name, bodies.symbol, modules.text_blob,
                    bodies.start, bodies.end
             FROM bodies
             JOIN instance_bodies ON instance_bodies.body_id = bodies.id
             JOIN selected_modules AS modules ON modules.id = bodies.module_id
             WHERE instance_bodies.instance_id = ?1 AND modules.stage = ?2
             ORDER BY modules.name, bodies.start",
        )?;
        let mut rows = statement.query(params![
            show.instance.id.as_str(),
            show.output.stage().as_str()
        ])?;
        let mut body_count = 0;

        while let Some(row) = rows.next()? {
            let body = stored_body_from_row(row)?;
            emit_show_event(
                on_event,
                ShowEvent::BodyStarted {
                    stage: show.output.stage(),
                    module: body.module,
                    symbol: body.symbol,
                },
            )?;
            let mut summary = crate::model::LlvmBodySummaryBuilder::new();
            self.read_blob_range_with(&body.text_blob, body.start, body.end, |text| {
                summary.push(&text);
                emit_show_event(on_event, ShowEvent::BodyChunk { text })
            })?;
            emit_show_event(
                on_event,
                ShowEvent::BodyFinished {
                    summary: summary.finish(),
                },
            )?;
            body_count += 1;
        }

        Ok(body_count)
    }

    pub(crate) fn stream_show_source(
        &self,
        show: &PreparedShow,
        on_event: &mut impl FnMut(ShowEvent) -> std::ops::ControlFlow<()>,
    ) -> Result<bool> {
        let source = self
            .connection
            .query_row(
                "SELECT sources.path, sources.blob, definitions.source_item_start,
                        definitions.source_item_end, definitions.source_item_line_start
                 FROM instances
                 JOIN definitions ON definitions.id = instances.definition_id
                 JOIN sources ON sources.capture_id = instances.capture_id
                             AND sources.path = definitions.source_path
                 WHERE instances.id = ?1
                   AND definitions.source_item_start IS NOT NULL",
                [show.instance.id.as_str()],
                |row| {
                    Ok(StoredSourceRange {
                        path: row.get(0)?,
                        blob: row.get(1)?,
                        start: row.get(2)?,
                        end: row.get(3)?,
                        start_line: integer_from_row(row, 4)?,
                    })
                },
            )
            .optional()?;
        let Some(source) = source else {
            return Ok(false);
        };

        emit_show_event(
            on_event,
            ShowEvent::SourceStarted {
                path: source.path,
                start_line: source.start_line,
            },
        )?;
        self.read_blob_range_with(&source.blob, source.start, source.end, |text| {
            emit_show_event(on_event, ShowEvent::SourceChunk { text })
        })?;
        emit_show_event(on_event, ShowEvent::SourceFinished)?;

        Ok(true)
    }

    pub(crate) fn show_remarks(
        &self,
        instance_prefix: &InstanceId,
        options: &RemarkOptions,
    ) -> Result<RemarkShowView> {
        let resolved = self.resolve_instance(instance_prefix)?;
        let instance = self.connection.query_row(
            &format!("{} WHERE instances.id = ?1", instance_select()),
            [resolved.instance_id.as_str()],
            instance_from_row,
        )?;
        let summary = self
            .capture_summary(&resolved.capture_id, CaptureDisposition::Captured)?
            .remarks;
        let mut statement = self.connection.prepare(
            "SELECT remark_files.name, remarks.ordinal, remarks.kind, remarks.unknown_kind,
                    remarks.pass_name, remarks.remark_name, remarks.function_symbol,
                    remarks.source_file, remarks.source_line, remarks.source_column,
                    remarks.hotness, remarks.arguments_json, remarks.message
             FROM remark_instances
             JOIN remarks ON remarks.id = remark_instances.remark_id
             JOIN remark_files ON remark_files.id = remarks.file_id
             WHERE remark_instances.instance_id = ?1
               AND (?2 IS NULL OR remarks.kind = ?2)
               AND (?3 IS NULL OR remarks.pass_name = ?3)
             ORDER BY remark_files.name, remarks.ordinal
             LIMIT ?4",
        )?;
        let kind = options.kind.map(RemarkKindFilter::name);
        let fetch_limit = sqlite_usize("remark result limit", options.limit.saturating_add(1))?;
        let mut remarks = statement
            .query_map(
                params![
                    resolved.instance_id.as_str(),
                    kind,
                    options.pass,
                    fetch_limit
                ],
                remark_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let truncated = remarks.len() > options.limit;
        remarks.truncate(options.limit);

        Ok(RemarkShowView {
            capture_id: resolved.capture_id,
            instance,
            summary,
            remarks,
            truncated,
            source: None,
        })
    }

    pub(crate) fn source_file(
        &self,
        capture_id: &CaptureId,
        location: &SourceLocation,
    ) -> Result<Option<StoredSource>> {
        let source = self
            .connection
            .query_row(
                "SELECT path, blob FROM sources WHERE capture_id = ?1 AND path = ?2",
                params![capture_id.as_str(), location.path],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        source
            .map(|(path, blob)| {
                Ok(StoredSource {
                    path,
                    bytes: self.read_blob(&blob)?,
                })
            })
            .transpose()
    }

    #[cfg(test)]
    fn publish_blob(&self, source: &Path) -> Result<String> {
        self.publish_blob_with_limit(source, None)
    }

    fn publish_blob_with_limit(
        &self,
        source: &Path,
        maximum_store_bytes: Option<u64>,
    ) -> Result<String> {
        self.ensure_storage_budget(maximum_store_bytes)?;
        let temporary = self
            .blobs
            .join(format!(".{}.tmp", uuid::Uuid::now_v7().simple()));
        let expected_digest = match compress_and_hash_file(source, &temporary) {
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
        if let Err(error) = self.ensure_storage_budget(maximum_store_bytes) {
            let _ = fs::remove_file(&temporary);

            return Err(error);
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
             SELECT blob FROM sources WHERE capture_id = ?1
             UNION
             SELECT blob FROM remark_files WHERE capture_id = ?1",
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
             UNION SELECT blob FROM sources
             UNION SELECT blob FROM remark_files",
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

    pub(crate) fn completed_capture(
        &self,
        capture_id: &CaptureId,
        request_key: &str,
        analysis_key: &AnalysisKey,
        disposition: CaptureDisposition,
    ) -> Result<Option<CaptureSummary>> {
        let summary = self
            .connection
            .query_row(
                "SELECT id, created_at_ms, rustc_release, llvm_version, target, profile,
                        (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id),
                        (SELECT COUNT(*) FROM modules WHERE capture_id = captures.id),
                        remarks_captured, remark_file_count, remark_record_count,
                        remark_linked_record_count
                 FROM captures WHERE id = ?1",
                [capture_id.as_str()],
                |row| summary_from_row(row, disposition),
            )
            .optional()
            .map_err(Error::from)?;
        if summary.is_none() {
            return Ok(None);
        }
        let matches = self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM capture_cache
                 WHERE request_key = ?1 AND capture_id = ?2 AND analysis_key = ?3
             )",
            params![request_key, capture_id.as_str(), analysis_key.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if !matches {
            return Err(Error::InvalidPendingEvidence {
                path: PathBuf::from("pending.json"),
                message: format!(
                    "completed capture does not match the retained request, got {capture_id}"
                ),
            });
        }

        Ok(summary)
    }

    fn capture_summary(
        &self,
        capture_id: &CaptureId,
        disposition: CaptureDisposition,
    ) -> Result<CaptureSummary> {
        self.connection
            .query_row(
                "SELECT id, created_at_ms, rustc_release, llvm_version, target, profile,
                        (SELECT COUNT(*) FROM instances WHERE capture_id = captures.id),
                        (SELECT COUNT(*) FROM modules WHERE capture_id = captures.id),
                        remarks_captured, remark_file_count, remark_record_count,
                        remark_linked_record_count
                 FROM captures WHERE id = ?1",
                [capture_id.as_str()],
                |row| summary_from_row(row, disposition),
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

    fn resolve_pending(&self, prefix: &PendingId) -> Result<PendingEntry> {
        let mut candidates = self
            .pending_entries()?
            .into_iter()
            .filter(|entry| entry.summary.id.as_str().starts_with(prefix.as_str()));
        let Some(candidate) = candidates.next() else {
            return Err(Error::UnknownPending {
                pending_id: prefix.clone(),
            });
        };
        let Some(other) = candidates.next() else {
            return Ok(candidate);
        };

        Err(ambiguous_identifier(
            "pending capture",
            prefix.as_str(),
            &[
                candidate.summary.id.to_string(),
                other.summary.id.to_string(),
            ],
        ))
    }

    fn pending_entries(&self) -> Result<Vec<PendingEntry>> {
        let directories = fs::read_dir(&self.pending)
            .map_err(|source| Error::filesystem("read", &self.pending, source))?;
        let mut entries = Vec::new();

        for directory in directories {
            let directory =
                directory.map_err(|source| Error::filesystem("read", &self.pending, source))?;
            let path = directory.path();
            if !directory
                .file_type()
                .map_err(|source| Error::filesystem("read metadata for", &path, source))?
                .is_dir()
                || !PendingCapture::exists(&path)
            {
                continue;
            }

            entries.push(PendingEntry {
                summary: PendingCapture::summary(&path)?,
                path,
            });
        }
        entries.sort_by(|left, right| left.summary.id.cmp(&right.summary.id));

        Ok(entries)
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

    fn query_instance_ids(
        &self,
        capture_id: &CaptureId,
        match_kind: InstanceMatch,
        options: &FindOptions,
    ) -> Result<Vec<InstanceId>> {
        let selection = match match_kind {
            InstanceMatch::Exact => {
                "instances.id IN (
                SELECT exact_definition.id
                FROM definitions AS exact_origin
                JOIN instances AS exact_definition
                  ON exact_definition.definition_id = exact_origin.id
                WHERE exact_origin.capture_id = ?1 AND exact_origin.path = ?2
                UNION
                SELECT exact_display.id FROM instances AS exact_display
                WHERE exact_display.capture_id = ?1 AND exact_display.display_name = ?2
                UNION
                SELECT exact_symbol.id FROM instances AS exact_symbol
                WHERE exact_symbol.capture_id = ?1 AND exact_symbol.compiler_symbol = ?2
            )"
            }
            InstanceMatch::Substring => {
                "instances.rowid IN (
                SELECT instance_search.rowid FROM instance_search
                WHERE instance_search MATCH ?2 AND instance_search.capture_id = ?1
            )"
            }
        };
        let sql = format!(
            "SELECT instances.id
             FROM instances JOIN definitions ON definitions.id = instances.definition_id
             WHERE instances.capture_id = ?1
               AND {selection}
               AND (?3 IS NULL OR definitions.crate_name = ?3)
               AND (?4 IS NULL OR definitions.path = ?4)
               AND (
                   ?5 IS NULL
                   OR (?5 = 'llvm-optimized' AND instances.llvm_definitions > 0)
                   OR (?5 = 'llvm-pre-optimization' AND instances.pre_opt_definitions > 0)
               )
             ORDER BY instances.display_name, definitions.path,
                      instances.compiler_symbol, instances.id
             LIMIT ?6"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let query = match match_kind {
            InstanceMatch::Exact => options.query.clone(),
            InstanceMatch::Substring => fts_literal_query(&options.query),
        };
        let available = options.available.map(|output| output.stage().as_str());
        let fetch_limit = sqlite_usize("find result limit", options.limit.saturating_add(1))?;
        let instances = statement
            .query_map(
                params![
                    capture_id.as_str(),
                    query,
                    options.crate_name,
                    options.definition,
                    available,
                    fetch_limit,
                ],
                |row| row.get::<_, InstanceId>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(instances)
    }

    fn hydrate_instances(&self, instance_ids: &[InstanceId]) -> Result<Vec<InstanceSummary>> {
        if instance_ids.is_empty() {
            return Ok(Vec::new());
        }

        let requested = instance_ids
            .iter()
            .enumerate()
            .map(|(position, _)| format!("(?{}, {position})", position + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "WITH requested(id, position) AS (VALUES {requested})
             {} JOIN requested ON requested.id = instances.id
             ORDER BY requested.position",
            instance_select()
        );
        let mut statement = self.connection.prepare(&sql)?;
        let instances = statement
            .query_map(
                rusqlite::params_from_iter(instance_ids.iter().map(InstanceId::as_str)),
                instance_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(instances)
    }

    fn verify_search_index(&self) -> Result<()> {
        let missing = self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM instances
                 LEFT JOIN instance_search ON instance_search.rowid = instances.rowid
                 JOIN definitions ON definitions.id = instances.definition_id
                 WHERE instance_search.rowid IS NULL
                    OR instance_search.instance_id != instances.id
                    OR instance_search.capture_id != instances.capture_id
                    OR instance_search.definition_path != definitions.path
                    OR instance_search.display_name != instances.display_name
                    OR instance_search.compiler_symbol != instances.compiler_symbol
             )",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        let extra = self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM instance_search
                 LEFT JOIN instances ON instances.rowid = instance_search.rowid
                 WHERE instances.rowid IS NULL
             )",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if missing || extra {
            return Err(Error::InvalidStoredData {
                message: "instance search index must correspond to every stored instance"
                    .to_owned(),
            });
        }

        Ok(())
    }

    fn read_blob(&self, digest: &str) -> Result<Vec<u8>> {
        let (path, expected) = self.verified_blob_path(digest)?;
        let mut decoder = blob_decoder(&path)?;
        let mut bytes = Vec::new();
        decoder
            .read_to_end(&mut bytes)
            .map_err(|source| invalid_blob("decompress", &path, source))?;

        verify_digest(&path, expected, blake3::hash(&bytes))?;

        Ok(bytes)
    }

    fn read_blob_range(&self, digest: &str, start: i64, end: i64) -> Result<String> {
        let mut text = String::new();
        self.read_blob_range_with(digest, start, end, |chunk| {
            text.push_str(&chunk);

            Ok(())
        })?;

        Ok(text)
    }

    fn read_blob_range_with(
        &self,
        digest: &str,
        start: i64,
        end: i64,
        mut on_chunk: impl FnMut(String) -> Result<()>,
    ) -> Result<()> {
        let (path, expected) = self.verified_blob_path(digest)?;
        verify_file_digest(&path, expected)?;
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
        let mut decoder = blob_decoder(&path)?;
        let skipped = io::copy(&mut decoder.by_ref().take(start), &mut io::sink())
            .map_err(|source| invalid_blob("decompress", &path, source))?;
        if skipped != start {
            return Err(Error::InvalidRange { path, start, end });
        }
        let mut reader = decoder.take(length);
        let mut buffer = vec![0_u8; TEXT_CHUNK_BYTES - 4];
        let mut pending = Vec::with_capacity(TEXT_CHUNK_BYTES);
        let mut read_bytes = 0_u64;

        loop {
            let bytes = reader
                .read(&mut buffer)
                .map_err(|source| invalid_blob("decompress", &path, source))?;
            if bytes == 0 {
                break;
            }

            read_bytes += bytes as u64;
            pending.extend_from_slice(&buffer[..bytes]);
            emit_utf8_prefix(&path, &mut pending, false, &mut on_chunk)?;
        }
        if read_bytes != length {
            return Err(Error::InvalidRange { path, start, end });
        }
        emit_utf8_prefix(&path, &mut pending, true, &mut on_chunk)
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

fn compress_and_hash_file(source: &Path, destination: &Path) -> Result<blake3::Hash> {
    let mut source_file =
        File::open(source).map_err(|error| Error::filesystem("open", source, error))?;
    let source_bytes = source_file
        .metadata()
        .map_err(|error| Error::filesystem("read metadata for", source, error))?
        .len();
    let destination_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| Error::filesystem("create", destination, error))?;
    let mut encoder =
        zstd::stream::write::Encoder::new(destination_file, BLOB_COMPRESSION_LEVEL)
            .map_err(|error| Error::filesystem("create compressor for", destination, error))?;
    encoder
        .include_checksum(true)
        .map_err(|error| Error::filesystem("configure compressor for", destination, error))?;
    encoder
        .include_contentsize(true)
        .map_err(|error| Error::filesystem("configure compressor for", destination, error))?;
    encoder
        .set_pledged_src_size(Some(source_bytes))
        .map_err(|error| Error::filesystem("configure compressor for", destination, error))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let length = source_file
            .read(&mut buffer)
            .map_err(|error| Error::filesystem("read", source, error))?;

        if length == 0 {
            break;
        }

        encoder
            .write_all(&buffer[..length])
            .map_err(|error| Error::filesystem("compress", destination, error))?;
        hasher.update(&buffer[..length]);
    }
    let destination_file = encoder
        .finish()
        .map_err(|error| Error::filesystem("finish compressing", destination, error))?;

    destination_file
        .sync_all()
        .map_err(|error| Error::filesystem("sync", destination, error))?;

    Ok(hasher.finalize())
}

fn verify_file_digest(path: &Path, expected: blake3::Hash) -> Result<()> {
    let mut decoder = blob_decoder(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let length = decoder
            .read(&mut buffer)
            .map_err(|source| invalid_blob("decompress", path, source))?;

        if length == 0 {
            break;
        }

        hasher.update(&buffer[..length]);
    }

    verify_digest(path, expected, hasher.finalize())
}

fn blob_decoder(path: &Path) -> Result<zstd::stream::read::Decoder<'static, BufReader<File>>> {
    let file = File::open(path).map_err(|source| Error::filesystem("open", path, source))?;

    zstd::stream::read::Decoder::new(file)
        .map_err(|source| invalid_blob("create decompressor for", path, source))
}

fn invalid_blob(operation: &'static str, path: &Path, source: io::Error) -> Error {
    Error::filesystem(
        operation,
        path,
        io::Error::new(io::ErrorKind::InvalidData, source),
    )
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

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
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

fn open_existing_lock_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
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

#[cfg(test)]
struct PublishedModule {
    name: String,
    provenance: cargo_ir::ArtifactProvenance,
    bitcode_blob: String,
    text_blob: String,
    bodies: Vec<cargo_ir::BodyRange>,
    declarations: Vec<cargo_ir::LlvmDeclaration>,
    aliases: Vec<cargo_ir::LlvmAlias>,
    selected: bool,
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
    remarks: RemarkCaptureSummary,
}

#[cfg(test)]
struct PublishedRemarkFile {
    name: String,
    blob: String,
    records: Vec<cargo_ir::OptimizationRemark>,
}

#[cfg(test)]
#[derive(Clone)]
struct IndexedBody {
    body_id: String,
    stage: Option<LlvmStage>,
    selected: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct AvailabilityCounts {
    llvm_definitions: usize,
    llvm_declarations: usize,
    llvm_aliases: usize,
    pre_opt_definitions: usize,
    pre_opt_declarations: usize,
    pre_opt_aliases: usize,
}

#[cfg(test)]
impl AvailabilityCounts {
    fn add_definition(&mut self, stage: Option<LlvmStage>) {
        match stage {
            Some(LlvmStage::Optimized) => self.llvm_definitions += 1,
            Some(LlvmStage::PreOptimization) => self.pre_opt_definitions += 1,
            None => {}
        }
    }

    fn add_declaration(&mut self, stage: Option<LlvmStage>) {
        match stage {
            Some(LlvmStage::Optimized) => self.llvm_declarations += 1,
            Some(LlvmStage::PreOptimization) => self.pre_opt_declarations += 1,
            None => {}
        }
    }

    fn add_alias(&mut self, stage: Option<LlvmStage>) {
        match stage {
            Some(LlvmStage::Optimized) => self.llvm_aliases += 1,
            Some(LlvmStage::PreOptimization) => self.pre_opt_aliases += 1,
            None => {}
        }
    }
}

#[cfg(test)]
struct IndexedEvidence {
    bodies: HashMap<String, Vec<IndexedBody>>,
    availability: HashMap<String, AvailabilityCounts>,
}

struct StoredBody {
    module: String,
    symbol: String,
    text_blob: String,
    start: i64,
    end: i64,
}

struct StoredSourceRange {
    path: String,

    blob: String,

    start: i64,

    end: i64,

    start_line: usize,
}

pub(crate) struct PreparedShow {
    pub(crate) capture_id: CaptureId,

    pub(crate) instance: InstanceSummary,

    pub(crate) output: CompilerOutput,
}

struct BlobEntry {
    path: PathBuf,
    digest: String,
    bytes: u64,
}

struct PendingEntry {
    path: PathBuf,

    summary: PendingSummary,
}

struct StagedCapture {
    path: PathBuf,

    connection: Connection,

    current_module: Option<String>,

    current_remark_file: Option<StagedRemarkFile>,

    events_since_budget_check: usize,
}

struct StagedRemarkFile {
    id: String,

    next_ordinal: usize,
}

struct CompletedStaging {
    path: PathBuf,

    remark_files: usize,

    remark_records: usize,

    linked_remark_records: usize,
}

impl StagedCapture {
    fn create(path: &Path) -> Result<Self> {
        if path.exists() {
            fs::remove_file(path).map_err(|source| Error::filesystem("remove", path, source))?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute_batch(
            "CREATE TABLE definitions(
                 key TEXT PRIMARY KEY,
                 id TEXT NOT NULL,
                 crate_name TEXT NOT NULL,
                 path TEXT NOT NULL,
                 source_path TEXT,
                 source_byte_start INTEGER,
                 source_byte_end INTEGER,
                 source_line_start INTEGER,
                 source_column_start INTEGER,
                 source_line_end INTEGER,
                 source_column_end INTEGER,
                 source_item_start INTEGER,
                 source_item_end INTEGER,
                 source_item_line_start INTEGER
             );
             CREATE TABLE instances(
                 key TEXT PRIMARY KEY,
                 id TEXT NOT NULL,
                 definition_id TEXT NOT NULL,
                 definition_path TEXT NOT NULL,
                 display_name TEXT NOT NULL,
                 compiler_symbol TEXT NOT NULL
             );
             CREATE INDEX instances_symbol ON instances(compiler_symbol);
             CREATE TABLE placements(
                 instance_id TEXT NOT NULL,
                 codegen_unit TEXT NOT NULL,
                 linkage TEXT NOT NULL,
                 visibility TEXT NOT NULL,
                 local_copy INTEGER NOT NULL,
                 size_estimate INTEGER NOT NULL,
                 PRIMARY KEY(instance_id, codegen_unit)
             );
             CREATE TABLE modules(
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 stage TEXT,
                 compiler_stage TEXT NOT NULL,
                 codegen_unit TEXT,
                 lto TEXT NOT NULL,
                 capture_method TEXT NOT NULL,
                 bitcode_blob TEXT NOT NULL,
                 text_blob TEXT NOT NULL
             );
             CREATE TABLE bodies(
                 id TEXT PRIMARY KEY,
                 module_id TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 start INTEGER NOT NULL,
                 end INTEGER NOT NULL
             );
             CREATE INDEX bodies_symbol ON bodies(symbol);
             CREATE TABLE declarations(
                 id TEXT PRIMARY KEY,
                 module_id TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 start INTEGER NOT NULL,
                 end INTEGER NOT NULL
             );
             CREATE INDEX declarations_symbol ON declarations(symbol);
             CREATE TABLE aliases(
                 id TEXT PRIMARY KEY,
                 module_id TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 target_kind TEXT NOT NULL,
                 target_symbol TEXT,
                 start INTEGER NOT NULL,
                 end INTEGER NOT NULL
             );
             CREATE INDEX aliases_symbol ON aliases(symbol);
             CREATE TABLE remark_files(
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 blob TEXT NOT NULL,
                 record_count INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE remarks(
                 id TEXT PRIMARY KEY,
                 file_id TEXT NOT NULL,
                 ordinal INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 unknown_kind TEXT,
                 pass_name TEXT NOT NULL,
                 remark_name TEXT NOT NULL,
                 function_symbol TEXT NOT NULL,
                 source_file TEXT,
                 source_line INTEGER,
                 source_column INTEGER,
                 hotness INTEGER,
                 arguments_json TEXT NOT NULL,
                 message TEXT NOT NULL,
                 UNIQUE(file_id, ordinal)
             );
             CREATE INDEX remarks_function ON remarks(function_symbol);
             CREATE TABLE sources(
                 path TEXT PRIMARY KEY,
                 blob TEXT NOT NULL
             );
             BEGIN IMMEDIATE;",
        )?;

        Ok(Self {
            path: path.to_owned(),
            connection,
            current_module: None,
            current_remark_file: None,
            events_since_budget_check: 0,
        })
    }

    fn push(
        &mut self,
        store: &Store,
        event: EvidenceEvent,
        maximum_store_bytes: Option<u64>,
    ) -> Result<()> {
        self.events_since_budget_check += 1;
        if self.events_since_budget_check == STORAGE_BUDGET_EVENT_INTERVAL {
            store.ensure_storage_budget(maximum_store_bytes)?;
            self.events_since_budget_check = 0;
        }

        match event {
            EvidenceEvent::Placement { record } => self.push_placement(record),
            EvidenceEvent::ModuleStarted { module } => {
                self.start_module(store, module, maximum_store_bytes)
            }
            EvidenceEvent::Body { body } => self.push_body(body),
            EvidenceEvent::Declaration { declaration } => self.push_declaration(declaration),
            EvidenceEvent::Alias { alias } => self.push_alias(alias),
            EvidenceEvent::RemarkFileStarted { file } => {
                self.start_remark_file(store, file, maximum_store_bytes)
            }
            EvidenceEvent::Remark { remark } => self.push_remark(remark),
        }
    }

    fn push_placement(&mut self, record: cargo_ir::CompilerPlacement) -> Result<()> {
        let definition_key = serde_json::to_string(&record.origin)?;
        let definition_id = format!("def_{}", uuid::Uuid::now_v7().simple());
        let source = record.origin.source.as_ref();
        self.connection.execute(
            "INSERT OR IGNORE INTO definitions(
                 key, id, crate_name, path, source_path, source_byte_start, source_byte_end,
                 source_line_start, source_column_start, source_line_end, source_column_end
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                definition_key,
                definition_id,
                record.origin.crate_name,
                record.origin.definition_path,
                source.map(|source| source.file_name.as_str()),
                optional_sqlite_integer(
                    "source byte start",
                    source.map(|source| source.byte_start)
                )?,
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
        let stored_definition_id = self.connection.query_row(
            "SELECT id FROM definitions WHERE key = ?1",
            [&definition_key],
            |row| row.get::<_, String>(0),
        )?;
        let instance_key =
            serde_json::to_string(&(&definition_key, &record.display_name, &record.raw_symbol))?;
        let instance_id = InstanceId::new();
        self.connection.execute(
            "INSERT OR IGNORE INTO instances(
                 key, id, definition_id, definition_path, display_name, compiler_symbol
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                instance_key,
                instance_id.as_str(),
                stored_definition_id,
                record.origin.definition_path,
                record.display_name,
                record.raw_symbol,
            ],
        )?;
        let stored_instance_id = self.connection.query_row(
            "SELECT id FROM instances WHERE key = ?1",
            [&instance_key],
            |row| row.get::<_, String>(0),
        )?;
        self.connection.execute(
            "INSERT OR REPLACE INTO placements(
                 instance_id, codegen_unit, linkage, visibility, local_copy, size_estimate
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                stored_instance_id,
                record.placement.codegen_unit,
                record.placement.linkage,
                record.placement.visibility,
                record.placement.local_copy,
                sqlite_usize("instance size estimate", record.placement.size_estimate)?,
            ],
        )?;

        Ok(())
    }

    fn start_module(
        &mut self,
        store: &Store,
        module: cargo_ir::ModuleStart,
        maximum_store_bytes: Option<u64>,
    ) -> Result<()> {
        let module_id = format!("mod_{}", uuid::Uuid::now_v7().simple());
        let bitcode_blob =
            store.publish_blob_with_limit(&module.bitcode_path, maximum_store_bytes)?;
        let text_blob = store.publish_blob_with_limit(&module.text_path, maximum_store_bytes)?;
        self.connection.execute(
            "INSERT INTO modules(
                 id, name, stage, compiler_stage, codegen_unit, lto, capture_method,
                 bitcode_blob, text_blob
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                module_id,
                module.name,
                module.provenance.stage.map(LlvmStage::as_str),
                module.provenance.compiler_stage,
                module.provenance.codegen_unit,
                lto_name(module.provenance.lto),
                capture_method_name(module.provenance.capture_method),
                bitcode_blob,
                text_blob,
            ],
        )?;
        self.current_module = Some(module_id);

        Ok(())
    }

    fn push_body(&mut self, body: cargo_ir::BodyRange) -> Result<()> {
        let module_id = self.current_module()?;
        self.connection.execute(
            "INSERT INTO bodies(id, module_id, symbol, start, end) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                format!("body_{}", uuid::Uuid::now_v7().simple()),
                module_id,
                body.raw_symbol,
                sqlite_integer("LLVM body start offset", body.start)?,
                sqlite_integer("LLVM body end offset", body.end)?,
            ],
        )?;

        Ok(())
    }

    fn push_declaration(&mut self, declaration: cargo_ir::LlvmDeclaration) -> Result<()> {
        let module_id = self.current_module()?;
        self.connection.execute(
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

        Ok(())
    }

    fn push_alias(&mut self, alias: cargo_ir::LlvmAlias) -> Result<()> {
        let module_id = self.current_module()?;
        let (target_kind, target_symbol) = match &alias.target {
            cargo_ir::AliasTarget::Symbol { raw_symbol } => ("symbol", Some(raw_symbol.as_str())),
            cargo_ir::AliasTarget::Expression => ("expression", None),
        };
        self.connection.execute(
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

        Ok(())
    }

    fn start_remark_file(
        &mut self,
        store: &Store,
        file: cargo_ir::RemarkFileStart,
        maximum_store_bytes: Option<u64>,
    ) -> Result<()> {
        let id = format!("remfile_{}", uuid::Uuid::now_v7().simple());
        let blob = store.publish_blob_with_limit(&file.raw_path, maximum_store_bytes)?;
        self.connection.execute(
            "INSERT INTO remark_files(id, name, blob) VALUES (?1, ?2, ?3)",
            params![id, file.name, blob],
        )?;
        self.current_remark_file = Some(StagedRemarkFile {
            id,
            next_ordinal: 0,
        });

        Ok(())
    }

    fn push_remark(&mut self, remark: cargo_ir::OptimizationRemark) -> Result<()> {
        let file = self
            .current_remark_file
            .as_mut()
            .ok_or_else(|| Error::InvalidStoredData {
                message: "a remark record must follow a remark-file event".to_owned(),
            })?;
        let ordinal = file.next_ordinal;
        file.next_ordinal = file.next_ordinal.saturating_add(1);
        let (kind, unknown_kind) = remark_kind_name(&remark.kind);
        let source = remark.source_location.as_ref();
        self.connection.execute(
            "INSERT INTO remarks(
                 id, file_id, ordinal, kind, unknown_kind, pass_name, remark_name,
                 function_symbol, source_file, source_line, source_column, hotness,
                 arguments_json, message
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                format!("rem_{}", uuid::Uuid::now_v7().simple()),
                file.id,
                sqlite_usize("remark ordinal", ordinal)?,
                kind,
                unknown_kind,
                remark.pass_name,
                remark.remark_name,
                remark.function,
                source.map(|source| source.file.as_str()),
                optional_sqlite_integer("remark source line", source.map(|source| source.line))?,
                optional_sqlite_integer(
                    "remark source column",
                    source.map(|source| source.column)
                )?,
                optional_sqlite_integer("remark hotness", remark.hotness)?,
                serde_json::to_string(&remark.arguments)?,
                remark.message,
            ],
        )?;
        self.connection.execute(
            "UPDATE remark_files SET record_count = record_count + 1 WHERE id = ?1",
            [&file.id],
        )?;

        Ok(())
    }

    fn push_source(
        &mut self,
        store: &Store,
        source: &crate::source::SourceEntry,
        maximum_store_bytes: Option<u64>,
    ) -> Result<()> {
        let blob = store.publish_blob_with_limit(&source.snapshot, maximum_store_bytes)?;
        let source_path = source.path.to_string_lossy();
        self.connection.execute(
            "INSERT INTO sources(path, blob) VALUES (?1, ?2)",
            params![source_path.as_ref(), blob],
        )?;
        for range in crate::source::source_item_ranges(&source.snapshot)? {
            self.connection.execute(
                "UPDATE definitions SET
                     source_item_start = ?1,
                     source_item_end = ?2,
                     source_item_line_start = ?3
                 WHERE source_path = ?4 AND source_byte_start = ?5 AND source_byte_end = ?6",
                params![
                    sqlite_usize("source item start", range.item.start)?,
                    sqlite_usize("source item end", range.item.end)?,
                    sqlite_usize("source item line start", range.start_line)?,
                    source_path.as_ref(),
                    sqlite_usize("definition source start", range.definition.start)?,
                    sqlite_usize("definition source end", range.definition.end)?,
                ],
            )?;
        }

        Ok(())
    }

    fn current_module(&self) -> Result<&str> {
        self.current_module
            .as_deref()
            .ok_or_else(|| Error::InvalidStoredData {
                message: "an LLVM symbol record must follow a module event".to_owned(),
            })
    }

    fn finish(self) -> Result<CompletedStaging> {
        let remark_files =
            self.connection
                .query_row("SELECT count(*) FROM remark_files", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        let remark_records =
            self.connection
                .query_row("SELECT count(*) FROM remarks", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        let linked_remark_records = self.connection.query_row(
            "SELECT count(*) FROM remarks
             WHERE EXISTS(
                 SELECT 1 FROM instances
                 WHERE instances.compiler_symbol = remarks.function_symbol
             )",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        self.connection.execute_batch("COMMIT")?;
        drop(self.connection);

        Ok(CompletedStaging {
            path: self.path,
            remark_files: stored_count("remark file count", remark_files)?,
            remark_records: stored_count("remark record count", remark_records)?,
            linked_remark_records: stored_count(
                "linked remark record count",
                linked_remark_records,
            )?,
        })
    }
}

fn stored_count(field: &str, value: i64) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidStoredData {
        message: format!("{field} must fit in usize, got {value}"),
    })
}

fn emit_show_event(
    on_event: &mut impl FnMut(ShowEvent) -> std::ops::ControlFlow<()>,
    event: ShowEvent,
) -> Result<()> {
    if on_event(event).is_break() {
        return Err(Error::ConsumerStopped);
    }

    Ok(())
}

fn emit_utf8_prefix(
    path: &Path,
    pending: &mut Vec<u8>,
    final_chunk: bool,
    on_chunk: &mut impl FnMut(String) -> Result<()>,
) -> Result<()> {
    let valid_bytes = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        Err(error) if error.error_len().is_none() && !final_chunk => error.valid_up_to(),
        Err(error) => {
            return Err(Error::filesystem(
                "decode UTF-8 from",
                path,
                io::Error::new(io::ErrorKind::InvalidData, error),
            ));
        }
    };
    if valid_bytes == 0 {
        return Ok(());
    }

    let chunk = String::from_utf8(pending.drain(..valid_bytes).collect()).map_err(|source| {
        Error::filesystem(
            "decode UTF-8 from",
            path,
            io::Error::new(io::ErrorKind::InvalidData, source),
        )
    })?;
    on_chunk(chunk)
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

fn validate_schema(connection: &Connection) -> Result<()> {
    let actual = connection.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))?;
    if actual == STORE_VERSION {
        return Ok(());
    }

    Err(Error::StoreVersion {
        expected: STORE_VERSION,
        actual,
    })
}

fn create_schema(connection: &mut Connection) -> Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE captures(
             id TEXT PRIMARY KEY,
             created_at_ms INTEGER NOT NULL,
             request_key TEXT NOT NULL,
             request_json TEXT NOT NULL,
             rustc_path TEXT NOT NULL,
             rustc_release TEXT NOT NULL,
             rustc_commit TEXT NOT NULL,
             rustc_host TEXT NOT NULL,
             llvm_version TEXT NOT NULL,
             rustc_sysroot TEXT NOT NULL,
             llvm_dis_path TEXT NOT NULL,
             target TEXT NOT NULL,
             profile TEXT NOT NULL,
             invocation_json TEXT NOT NULL,
             remarks_captured INTEGER NOT NULL DEFAULT 0 CHECK(remarks_captured IN (0, 1)),
             remark_file_count INTEGER NOT NULL DEFAULT 0 CHECK(remark_file_count >= 0),
             remark_record_count INTEGER NOT NULL DEFAULT 0 CHECK(remark_record_count >= 0),
             remark_linked_record_count INTEGER NOT NULL DEFAULT 0 CHECK(
                 remark_linked_record_count >= 0
                 AND remark_linked_record_count <= remark_record_count
             ),
             CHECK(remarks_captured = 1 OR (
                 remark_file_count = 0
                 AND remark_record_count = 0
                 AND remark_linked_record_count = 0
             ))
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
             source_column_end INTEGER,
             source_item_start INTEGER,
             source_item_end INTEGER,
             source_item_line_start INTEGER,
             CHECK(
                 (source_item_start IS NULL AND source_item_end IS NULL
                  AND source_item_line_start IS NULL)
                 OR (source_item_start IS NOT NULL AND source_item_end IS NOT NULL
                     AND source_item_line_start IS NOT NULL)
             )
         );
         CREATE INDEX definitions_path ON definitions(capture_id, path, id);
         CREATE INDEX definitions_identity ON definitions(capture_id, crate_name, path, id);
         CREATE TABLE instances(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             definition_id TEXT NOT NULL REFERENCES definitions(id) ON DELETE CASCADE,
             display_name TEXT NOT NULL,
             compiler_symbol TEXT NOT NULL,
             llvm_definitions INTEGER NOT NULL DEFAULT 0 CHECK(llvm_definitions >= 0),
             llvm_declarations INTEGER NOT NULL DEFAULT 0 CHECK(llvm_declarations >= 0),
             llvm_aliases INTEGER NOT NULL DEFAULT 0 CHECK(llvm_aliases >= 0),
             pre_opt_definitions INTEGER NOT NULL DEFAULT 0 CHECK(pre_opt_definitions >= 0),
             pre_opt_declarations INTEGER NOT NULL DEFAULT 0 CHECK(pre_opt_declarations >= 0),
             pre_opt_aliases INTEGER NOT NULL DEFAULT 0 CHECK(pre_opt_aliases >= 0)
         );
         CREATE INDEX instances_definition ON instances(capture_id, definition_id, id);
         CREATE INDEX instances_display_name ON instances(capture_id, display_name, id);
         CREATE INDEX instances_compiler_symbol ON instances(capture_id, compiler_symbol, id);
         CREATE INDEX instances_llvm_available ON instances(capture_id, id)
             WHERE llvm_definitions > 0;
         CREATE INDEX instances_pre_opt_available ON instances(capture_id, id)
             WHERE pre_opt_definitions > 0;
         CREATE VIRTUAL TABLE instance_search USING fts5(
             instance_id UNINDEXED,
             capture_id UNINDEXED,
             definition_path,
             display_name,
             compiler_symbol,
             tokenize = 'trigram case_sensitive 1'
         );
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
         CREATE TABLE remark_files(
             id TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             name TEXT NOT NULL,
             blob TEXT NOT NULL,
             record_count INTEGER NOT NULL CHECK(record_count >= 0),
             UNIQUE(capture_id, name)
         );
         CREATE TABLE remarks(
             id TEXT PRIMARY KEY,
             file_id TEXT NOT NULL REFERENCES remark_files(id) ON DELETE CASCADE,
             ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
             kind TEXT NOT NULL CHECK(kind IN (
                 'passed', 'missed', 'analysis', 'analysis-fp-commute',
                 'analysis-aliasing', 'failure', 'unknown'
             )),
             unknown_kind TEXT,
             pass_name TEXT NOT NULL,
             remark_name TEXT NOT NULL,
             function_symbol TEXT NOT NULL,
             source_file TEXT,
             source_line INTEGER,
             source_column INTEGER,
             hotness INTEGER,
             arguments_json TEXT NOT NULL,
             message TEXT NOT NULL,
             UNIQUE(file_id, ordinal),
             CHECK((kind = 'unknown') = (unknown_kind IS NOT NULL)),
             CHECK(
                 (source_file IS NULL AND source_line IS NULL AND source_column IS NULL)
                 OR (source_file IS NOT NULL AND source_line IS NOT NULL
                     AND source_column IS NOT NULL)
             )
         );
         CREATE INDEX remarks_function ON remarks(function_symbol);
         CREATE INDEX remarks_filter ON remarks(kind, pass_name);
         CREATE TABLE remark_instances(
             remark_id TEXT NOT NULL REFERENCES remarks(id) ON DELETE CASCADE,
             instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
             PRIMARY KEY(remark_id, instance_id)
         );
         CREATE INDEX remark_instances_instance
             ON remark_instances(instance_id, remark_id);
         CREATE TABLE sources(
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             path TEXT NOT NULL,
             blob TEXT NOT NULL,
             PRIMARY KEY(capture_id, path)
         );
         CREATE TABLE capture_cache(
             request_key TEXT PRIMARY KEY,
             capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
             analysis_key TEXT NOT NULL
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
             id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
             rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
             profile, invocation_json, remarks_captured, remark_file_count,
             remark_record_count, remark_linked_record_count
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
             ?15, ?16, ?17, ?18
         )",
        params![
            capture.capture_id.as_str(),
            created_at_ms,
            capture.request_key,
            capture.request_json,
            capture.toolchain.rustc.to_string_lossy(),
            capture.toolchain.release,
            capture.toolchain.commit_hash,
            capture.toolchain.host,
            capture.toolchain.llvm_version,
            capture.toolchain.sysroot.to_string_lossy(),
            capture.toolchain.llvm_dis.to_string_lossy(),
            capture.target,
            capture_profile_name(capture.spec.capture_profile),
            capture.invocation_json,
            capture.remarks.state != RemarkEvidenceState::NotCaptured,
            sqlite_usize("remark file count", capture.remarks.files)?,
            sqlite_usize("remark record count", capture.remarks.records)?,
            sqlite_usize("linked remark record count", capture.remarks.linked_records)?,
        ],
    )?;

    Ok(())
}

#[cfg(test)]
fn insert_modules(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    modules: &[PublishedModule],
) -> Result<IndexedEvidence> {
    let mut body_index: HashMap<String, Vec<IndexedBody>> = HashMap::new();
    let mut availability: HashMap<String, AvailabilityCounts> = HashMap::new();
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
                .push(IndexedBody {
                    body_id,
                    stage: module.provenance.stage,
                    selected: module.selected,
                });
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
            if module.selected {
                availability
                    .entry(declaration.raw_symbol.clone())
                    .or_default()
                    .add_declaration(module.provenance.stage);
            }
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
            if module.selected {
                availability
                    .entry(alias.raw_symbol.clone())
                    .or_default()
                    .add_alias(module.provenance.stage);
            }
        }
    }

    for (alias, target) in direct_aliases {
        if let Some(bodies) = body_index.get(&target).cloned() {
            body_index.entry(alias).or_default().extend(bodies);
        }
    }

    Ok(IndexedEvidence {
        bodies: body_index,
        availability,
    })
}

fn update_streamed_availability(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
) -> Result<()> {
    transaction.execute(
        "UPDATE instances SET
             llvm_definitions = (
                 SELECT count(*) FROM instance_bodies
                 JOIN bodies ON bodies.id = instance_bodies.body_id
                 JOIN selected_modules AS modules ON modules.id = bodies.module_id
                 WHERE instance_bodies.instance_id = instances.id
                   AND modules.stage = 'llvm-optimized'
             ),
             pre_opt_definitions = (
                 SELECT count(*) FROM instance_bodies
                 JOIN bodies ON bodies.id = instance_bodies.body_id
                 JOIN selected_modules AS modules ON modules.id = bodies.module_id
                 WHERE instance_bodies.instance_id = instances.id
                   AND modules.stage = 'llvm-pre-optimization'
             ),
             llvm_declarations = (
                 SELECT count(*) FROM declarations
                 JOIN selected_modules AS modules ON modules.id = declarations.module_id
                 WHERE declarations.symbol = instances.compiler_symbol
                   AND modules.capture_id = instances.capture_id
                   AND modules.stage = 'llvm-optimized'
             ),
             pre_opt_declarations = (
                 SELECT count(*) FROM declarations
                 JOIN selected_modules AS modules ON modules.id = declarations.module_id
                 WHERE declarations.symbol = instances.compiler_symbol
                   AND modules.capture_id = instances.capture_id
                   AND modules.stage = 'llvm-pre-optimization'
             ),
             llvm_aliases = (
                 SELECT count(*) FROM aliases
                 JOIN selected_modules AS modules ON modules.id = aliases.module_id
                 WHERE aliases.symbol = instances.compiler_symbol
                   AND modules.capture_id = instances.capture_id
                   AND modules.stage = 'llvm-optimized'
             ),
             pre_opt_aliases = (
                 SELECT count(*) FROM aliases
                 JOIN selected_modules AS modules ON modules.id = aliases.module_id
                 WHERE aliases.symbol = instances.compiler_symbol
                   AND modules.capture_id = instances.capture_id
                   AND modules.stage = 'llvm-pre-optimization'
             )
         WHERE capture_id = ?1",
        [capture_id.as_str()],
    )?;

    Ok(())
}

fn associate_streamed_bodies(transaction: &Transaction<'_>, capture_id: &CaptureId) -> Result<()> {
    transaction.execute(
        "INSERT INTO instance_bodies(instance_id, body_id)
         SELECT instances.id, bodies.id
         FROM instances
         JOIN bodies ON bodies.symbol = instances.compiler_symbol
         JOIN selected_modules AS modules ON modules.id = bodies.module_id
                                         AND modules.capture_id = instances.capture_id
         WHERE instances.capture_id = ?1
         UNION
         SELECT instances.id, bodies.id
         FROM instances
         JOIN aliases ON aliases.symbol = instances.compiler_symbol
                     AND aliases.target_kind = 'symbol'
         JOIN selected_modules AS alias_modules
           ON alias_modules.id = aliases.module_id
          AND alias_modules.capture_id = instances.capture_id
         JOIN bodies ON bodies.symbol = aliases.target_symbol
         JOIN selected_modules AS body_modules
           ON body_modules.id = bodies.module_id
          AND body_modules.capture_id = instances.capture_id
         WHERE instances.capture_id = ?1",
        [capture_id.as_str()],
    )?;

    Ok(())
}

#[cfg(test)]
fn insert_instances(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    instances: &[cargo_ir::CompilerInstance],
    evidence_index: &IndexedEvidence,
) -> Result<HashMap<String, Vec<InstanceId>>> {
    let mut definitions: HashMap<String, String> = HashMap::new();
    let mut instances_by_symbol: HashMap<String, Vec<InstanceId>> = HashMap::new();

    for instance in instances {
        let instance_id = InstanceId::new();
        let bodies = bodies_for_instance(instance, &evidence_index.bodies);
        let mut availability = evidence_index
            .availability
            .get(&instance.raw_symbol)
            .copied()
            .unwrap_or_default();
        for body in bodies.iter().filter(|body| body.selected) {
            availability.add_definition(body.stage);
        }
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
                 id, capture_id, definition_id, display_name, compiler_symbol,
                 llvm_definitions, llvm_declarations, llvm_aliases,
                 pre_opt_definitions, pre_opt_declarations, pre_opt_aliases
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                instance_id.as_str(),
                capture_id.as_str(),
                definition_id,
                instance.display_name,
                instance.raw_symbol,
                sqlite_usize("optimized LLVM definitions", availability.llvm_definitions)?,
                sqlite_usize(
                    "optimized LLVM declarations",
                    availability.llvm_declarations
                )?,
                sqlite_usize("optimized LLVM aliases", availability.llvm_aliases)?,
                sqlite_usize(
                    "pre-optimization LLVM definitions",
                    availability.pre_opt_definitions
                )?,
                sqlite_usize(
                    "pre-optimization LLVM declarations",
                    availability.pre_opt_declarations
                )?,
                sqlite_usize(
                    "pre-optimization LLVM aliases",
                    availability.pre_opt_aliases
                )?,
            ],
        )?;
        instances_by_symbol
            .entry(instance.raw_symbol.clone())
            .or_default()
            .push(instance_id.clone());
        let instance_rowid = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO instance_search(
                 rowid, instance_id, capture_id, definition_path, display_name, compiler_symbol
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                instance_rowid,
                instance_id.as_str(),
                capture_id.as_str(),
                instance.origin.definition_path,
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

    Ok(instances_by_symbol)
}

#[cfg(test)]
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

#[cfg(test)]
fn bodies_for_instance<'a>(
    instance: &cargo_ir::CompilerInstance,
    body_index: &'a HashMap<String, Vec<IndexedBody>>,
) -> &'a [IndexedBody] {
    body_index
        .get(&instance.raw_symbol)
        .map_or(&[], Vec::as_slice)
}

#[cfg(test)]
fn insert_remarks(
    transaction: &Transaction<'_>,
    capture_id: &CaptureId,
    files: &[PublishedRemarkFile],
    instances_by_symbol: &HashMap<String, Vec<InstanceId>>,
) -> Result<()> {
    for file in files {
        let file_id = format!("remfile_{}", uuid::Uuid::now_v7().simple());
        transaction.execute(
            "INSERT INTO remark_files(id, capture_id, name, blob, record_count)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file_id,
                capture_id.as_str(),
                file.name,
                file.blob,
                sqlite_usize("remark file record count", file.records.len())?,
            ],
        )?;

        for (ordinal, remark) in file.records.iter().enumerate() {
            let remark_id = format!("rem_{}", uuid::Uuid::now_v7().simple());
            let (kind, unknown_kind) = remark_kind_name(&remark.kind);
            let source = remark.source_location.as_ref();
            let arguments_json = serde_json::to_string(&remark.arguments)?;
            transaction.execute(
                "INSERT INTO remarks(
                     id, file_id, ordinal, kind, unknown_kind, pass_name, remark_name,
                     function_symbol, source_file, source_line, source_column, hotness,
                     arguments_json, message
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    remark_id,
                    file_id,
                    sqlite_usize("remark ordinal", ordinal)?,
                    kind,
                    unknown_kind,
                    remark.pass_name,
                    remark.remark_name,
                    remark.function,
                    source.map(|source| source.file.as_str()),
                    optional_sqlite_integer(
                        "remark source line",
                        source.map(|source| source.line)
                    )?,
                    optional_sqlite_integer(
                        "remark source column",
                        source.map(|source| source.column)
                    )?,
                    optional_sqlite_integer("remark hotness", remark.hotness)?,
                    arguments_json,
                    remark.message,
                ],
            )?;

            if let Some(instances) = instances_by_symbol.get(&remark.function) {
                for instance_id in instances {
                    transaction.execute(
                        "INSERT INTO remark_instances(remark_id, instance_id) VALUES (?1, ?2)",
                        params![remark_id, instance_id.as_str()],
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn remark_kind_name(kind: &cargo_ir::RemarkKind) -> (&'static str, Option<&str>) {
    match kind {
        cargo_ir::RemarkKind::Passed => ("passed", None),
        cargo_ir::RemarkKind::Missed => ("missed", None),
        cargo_ir::RemarkKind::Analysis => ("analysis", None),
        cargo_ir::RemarkKind::AnalysisFpCommute => ("analysis-fp-commute", None),
        cargo_ir::RemarkKind::AnalysisAliasing => ("analysis-aliasing", None),
        cargo_ir::RemarkKind::Failure => ("failure", None),
        cargo_ir::RemarkKind::Unknown { tag } => ("unknown", Some(tag)),
    }
}

fn summary_from_row(
    row: &rusqlite::Row<'_>,
    disposition: CaptureDisposition,
) -> rusqlite::Result<CaptureSummary> {
    let profile = capture_profile_from_name(row.get::<_, String>(5)?.as_str(), 5)?;
    let remarks_captured = row.get::<_, bool>(8)?;
    let remark_files = integer_from_row(row, 9)?;
    let remark_records = integer_from_row(row, 10)?;
    let linked_remark_records = integer_from_row(row, 11)?;

    Ok(CaptureSummary {
        id: row.get(0)?,
        created_at_ms: integer_from_row(row, 1)?,
        disposition,
        rustc_release: row.get(2)?,
        llvm_version: row.get(3)?,
        target: row.get(4)?,
        capture_profile: profile,
        instance_count: integer_from_row(row, 6)?,
        module_count: integer_from_row(row, 7)?,
        remarks: remark_summary(
            remarks_captured,
            remark_files,
            remark_records,
            linked_remark_records,
        ),
    })
}

fn remark_summary(
    captured: bool,
    files: usize,
    records: usize,
    linked_records: usize,
) -> RemarkCaptureSummary {
    let state = if !captured {
        RemarkEvidenceState::NotCaptured
    } else if records == 0 {
        RemarkEvidenceState::CapturedEmpty
    } else {
        RemarkEvidenceState::Captured
    };

    RemarkCaptureSummary {
        state,
        files,
        records,
        linked_records,
        unlinked_records: records.saturating_sub(linked_records),
    }
}

fn validate_request_key(value: &str) -> Result<()> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid {
        return Err(Error::InvalidPendingEvidence {
            path: PathBuf::from("pending.json"),
            message: format!(
                "request key must contain 64 lowercase hexadecimal characters, got {value}"
            ),
        });
    }

    Ok(())
}

fn pending_entries(root: &Path) -> Result<(usize, u64)> {
    let mut pending = 0;

    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|source| Error::filesystem("walk", root, source.into()))?;
        if entry.file_type().is_file() && entry.file_name() == "pending.json" {
            pending += 1;
        }
    }

    Ok((pending, directory_bytes(root)?))
}

fn directory_bytes(root: &Path) -> Result<u64> {
    let mut bytes = 0_u64;

    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|source| Error::filesystem("walk", root, source.into()))?;
        if !entry.file_type().is_file() {
            continue;
        }

        bytes = bytes.saturating_add(
            entry
                .metadata()
                .map_err(|source| {
                    Error::filesystem("read metadata for", entry.path(), source.into())
                })?
                .len(),
        );
    }

    Ok(bytes)
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

fn remark_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemarkView> {
    let kind_name = row.get::<_, String>(2)?;
    let unknown_kind = row.get::<_, Option<String>>(3)?;
    let kind = remark_kind_from_name(&kind_name, unknown_kind, 2)?;
    let source_file = row.get::<_, Option<String>>(7)?;
    let source_location = match source_file {
        Some(file) => Some(cargo_ir::RemarkSourceLocation {
            file,
            line: integer_from_row(row, 8)?,
            column: integer_from_row(row, 9)?,
        }),
        None => None,
    };
    let arguments_json = row.get::<_, String>(11)?;
    let arguments = serde_json::from_str(&arguments_json).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(11, Type::Text, Box::new(source))
    })?;

    Ok(RemarkView {
        file: row.get(0)?,
        ordinal: integer_from_row(row, 1)?,
        kind,
        pass_name: row.get(4)?,
        remark_name: row.get(5)?,
        function: row.get(6)?,
        source_location,
        hotness: optional_integer_from_row(row, 10)?,
        arguments,
        message: row.get(12)?,
    })
}

fn remark_kind_from_name(
    value: &str,
    unknown: Option<String>,
    index: usize,
) -> rusqlite::Result<cargo_ir::RemarkKind> {
    match (value, unknown) {
        ("passed", None) => Ok(cargo_ir::RemarkKind::Passed),
        ("missed", None) => Ok(cargo_ir::RemarkKind::Missed),
        ("analysis", None) => Ok(cargo_ir::RemarkKind::Analysis),
        ("analysis-fp-commute", None) => Ok(cargo_ir::RemarkKind::AnalysisFpCommute),
        ("analysis-aliasing", None) => Ok(cargo_ir::RemarkKind::AnalysisAliasing),
        ("failure", None) => Ok(cargo_ir::RemarkKind::Failure),
        ("unknown", Some(tag)) => Ok(cargo_ir::RemarkKind::Unknown { tag }),
        _ => Err(invalid_stored_text(index, "remark kind", value)),
    }
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
            instances.compiler_symbol,
            definitions.source_path, definitions.source_byte_start, definitions.source_byte_end,
            definitions.source_line_start, definitions.source_column_start,
            definitions.source_line_end, definitions.source_column_end,
            instances.llvm_definitions, instances.llvm_declarations, instances.llvm_aliases,
            instances.pre_opt_definitions, instances.pre_opt_declarations,
            instances.pre_opt_aliases
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

fn optional_integer_from_row<T>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>>
where
    T: TryFrom<i64>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            T::try_from(value).map_err(|source| {
                rusqlite::Error::FromSqlConversionFailure(index, Type::Integer, Box::new(source))
            })
        })
        .transpose()
}

fn instance_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstanceSummary> {
    let compiler_symbol = row.get::<_, String>(4)?;
    let source_path = row.get::<_, Option<String>>(5)?;
    let source = match source_path {
        Some(path) => Some(SourceLocation {
            path,
            byte_start: integer_from_row(row, 6)?,
            byte_end: integer_from_row(row, 7)?,
            line_start: integer_from_row(row, 8)?,
            column_start: integer_from_row(row, 9)?,
            line_end: integer_from_row(row, 10)?,
            column_end: integer_from_row(row, 11)?,
        }),
        None => None,
    };
    let symbol_fingerprint = blake3::hash(compiler_symbol.as_bytes()).to_hex()[..12].to_owned();

    Ok(InstanceSummary {
        id: row.get(0)?,
        crate_name: row.get(1)?,
        definition: row.get(2)?,
        display_name: row.get(3)?,
        compiler_symbol,
        symbol_fingerprint,
        source,
        availability: vec![
            OutputAvailability {
                output: CompilerOutput::Llvm,
                definitions: integer_from_row(row, 12)?,
                declarations: integer_from_row(row, 13)?,
                aliases: integer_from_row(row, 14)?,
            },
            OutputAvailability {
                output: CompilerOutput::LlvmPreOpt,
                definitions: integer_from_row(row, 15)?,
                declarations: integer_from_row(row, 16)?,
                aliases: integer_from_row(row, 17)?,
            },
        ],
    })
}

fn fts_literal_query(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
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

pub(crate) const LEGACY_STORE_ENTRIES: &[&str] = &[
    "catalog.sqlite",
    "catalog.sqlite-shm",
    "catalog.sqlite-wal",
    "blobs",
    "pending",
    "staging",
    "work",
];

fn reject_legacy_store(optic: &Path) -> Result<()> {
    for entry in LEGACY_STORE_ENTRIES {
        let path = optic.join(entry);
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(Error::LegacyStore { path }),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::filesystem("read metadata for", path, source));
            }
        }
    }

    Ok(())
}

fn create_private_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(Error::filesystem(
                "create directory at",
                path,
                io::Error::new(io::ErrorKind::AlreadyExists, "path is not a directory"),
            ));
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| Error::filesystem("create", path, source))?;
        }
        Err(source) => return Err(Error::filesystem("read metadata for", path, source)),
    }

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
    use std::fs::{self, OpenOptions};
    use std::io;
    use std::path::PathBuf;

    use cargo_ir::{
        ArtifactProvenance, BodyRange, CaptureInvocation, CaptureMethod, CommandInvocation,
        CompiledCapture, CompilerInstance, DefinitionOrigin, LtoScope, Toolchain, UnstableAccess,
        UnstableAccessMechanism,
    };
    use fs2::FileExt as _;
    use rusqlite::Connection;

    use super::{
        IndexedBody, PublishedModule, PublishedRemarkFile, STORE_VERSION, Store,
        associate_streamed_bodies, bodies_for_instance, insert_instances, insert_modules,
        insert_remarks, lock_workspace_exclusive, lock_workspace_shared, unique_prefix_length,
        update_streamed_availability,
    };
    use crate::{
        BuildSpec, CaptureDisposition, CaptureId, CompilerOutput, Error, FindMatchKind,
        FindOptions, InstanceId, PendingId, RemarkEvidenceState, RemarkKindFilter, RemarkOptions,
    };

    fn write_pending_capture(store: &Store, request_key: &str, capture_id: &str) -> PendingId {
        let directory = store.pending.join(request_key);
        fs::create_dir(&directory).expect("the test can create a pending-capture directory");
        let request = cargo_ir::BuildRequest {
            workspace_root: PathBuf::from("workspace"),
            manifest_path: None,
            package: None,
            target: None,
            profile: None,
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
            target_triple: None,
            locked: false,
            offline: false,
            frozen: false,
            capture_profile: cargo_ir::CaptureProfile::Faithful,
            capture_remarks: false,
            analysis_directory: PathBuf::from("analysis"),
        };
        let compilation = CompiledCapture {
            invocation: CaptureInvocation {
                request,
                cargo: CommandInvocation {
                    program: "cargo".to_owned(),
                    arguments: Vec::new(),
                },
                rustc: None,
                wrapper_chain: Vec::new(),
                environment: Vec::new(),
                injected_rustc_arguments: Vec::new(),
                unstable_access: UnstableAccess {
                    mechanism: UnstableAccessMechanism::RustcBootstrap,
                    authorized_scopes: Vec::new(),
                },
            },
            toolchain: Toolchain {
                rustc: PathBuf::from("rustc"),
                release: "test-rustc".to_owned(),
                commit_hash: "0".repeat(40),
                host: "test-host".to_owned(),
                llvm_version: "test-llvm".to_owned(),
                sysroot: PathBuf::from("sysroot"),
                rustc_private_lib: PathBuf::from("rustc-private"),
                llvm_dis: PathBuf::from("llvm-dis"),
                rustup_toolchain: None,
            },
        };
        let marker = serde_json::json!({
            "version": 1,
            "request_key": request_key,
            "capture_id": capture_id,
            "analysis_key": "2".repeat(32),
            "spec": BuildSpec::default(),
            "compilation": compilation,
            "sources": {
                "entries": [],
                "cache_inputs": [],
            },
        });
        fs::write(
            directory.join("pending.json"),
            serde_json::to_vec(&marker).expect("the pending marker can be encoded"),
        )
        .expect("the test can write a pending marker");
        fs::write(directory.join("artifact.bc"), b"bitcode")
            .expect("the test can write a retained artifact");
        let capture_id = capture_id
            .parse::<CaptureId>()
            .expect("the test capture ID is valid");

        PendingId::from_capture(&capture_id)
    }

    #[test]
    fn creates_the_store_below_the_retained_optic_root() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let _store = Store::open(temporary.path()).expect("the test can open a store");

        for path in [
            ".optic/locks/operation.lock",
            ".optic/locks/schema.lock",
            ".optic/locks/writer.lock",
            ".optic/locks/evidence.lock",
            ".optic/store/catalog.sqlite",
            ".optic/store/blobs",
            ".optic/store/pending",
            ".optic/store/work",
        ] {
            assert!(temporary.path().join(path).exists(), "missing {path}");
        }
        assert!(!temporary.path().join(".optic.lock").exists());
    }

    #[test]
    fn opens_an_existing_store_read_only() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        drop(Store::open(temporary.path()).expect("the test can create a store"));
        let optic_dir = temporary.path().join(".optic");
        let store = Store::open_read_only(&optic_dir).expect("the test can read the store");

        assert!(
            store
                .captures()
                .expect("the catalog is readable")
                .is_empty()
        );
        assert!(matches!(
            store.lock_writer(),
            Err(Error::ReadOnlyStore { operation: "write evidence", path })
                if path == optic_dir
        ));
        assert!(
            store
                .connection
                .execute("DELETE FROM captures", [])
                .is_err()
        );
    }

    #[test]
    fn read_only_open_does_not_create_a_missing_store() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let optic_dir = temporary.path().join("missing/.optic");

        assert!(Store::open_read_only(&optic_dir).is_err());
        assert!(!optic_dir.exists());
    }

    #[test]
    fn read_only_open_does_not_initialize_an_empty_catalog() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        drop(Store::open(temporary.path()).expect("the test can create a store"));
        let optic_dir = temporary.path().join(".optic");
        let catalog = Connection::open(optic_dir.join("store/catalog.sqlite"))
            .expect("the test can edit the catalog version");
        catalog
            .pragma_update(None, "user_version", 0)
            .expect("the test can clear the catalog version");
        drop(catalog);

        assert!(matches!(
            Store::open_read_only(&optic_dir),
            Err(Error::StoreVersion {
                expected: STORE_VERSION,
                actual: 0,
            })
        ));
    }

    #[test]
    fn status_counts_recoverable_pending_evidence() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let pending = store.pending.join("0".repeat(64));
        fs::create_dir(&pending).expect("the test can create a pending request");
        fs::write(pending.join("artifact.bc"), b"bitcode")
            .expect("the test can create a retained artifact");
        fs::write(pending.join("pending.json"), b"marker")
            .expect("the test can create a pending marker");

        let status = store.status().expect("the test can read store status");

        assert_eq!(status.pending, 1);
        assert_eq!(status.pending_bytes, 13);
        assert!(status.retained_bytes >= status.pending_bytes);
        assert!(status.available_bytes > status.minimum_available_bytes);
        assert!(status.maximum_bytes > 0);
    }

    #[test]
    fn status_separates_referenced_and_reclaimable_blob_bytes() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        fs::write(&source, "define void @kernel() {}\n".repeat(100))
            .expect("the test can write source evidence");
        let digest = store
            .publish_blob(&source)
            .expect("the store can publish unreferenced evidence");
        let blob_bytes = fs::metadata(store.blob_path(&digest))
            .expect("the compressed blob has metadata")
            .len();

        let status = store.status().expect("the test can read store status");

        assert_eq!(status.blobs, 1);
        assert_eq!(status.blob_bytes, blob_bytes);
        assert_eq!(status.referenced_blob_bytes, 0);
        assert_eq!(status.unreferenced_blob_bytes, blob_bytes);
        assert!(status.retained_bytes >= blob_bytes);
    }

    #[test]
    fn rejects_new_evidence_at_the_command_storage_limit() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");

        assert!(matches!(
            store.ensure_storage_budget(Some(0)),
            Err(Error::StoreBudgetExceeded {
                maximum_bytes: 0,
                ..
            })
        ));
    }

    #[test]
    fn lists_inspects_and_removes_pending_captures_by_prefix() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let first = write_pending_capture(
            &store,
            &"0".repeat(64),
            "cap_00000000000000000000000000000000",
        );
        let second = write_pending_capture(
            &store,
            &"1".repeat(64),
            "cap_01000000000000000000000000000000",
        );

        let pending = store.pending().expect("the pending captures can be listed");
        let inspected = store
            .pending_summary(&"pen_00".parse().expect("the pending prefix is valid"))
            .expect("the unique pending prefix resolves");
        let unique = store
            .unique_pending_prefix(&first)
            .expect("the full pending ID has a unique prefix");
        let removed = store
            .remove_pending(&"pen_00".parse().expect("the pending prefix is valid"))
            .expect("the pending capture can be removed");

        assert_eq!(
            pending
                .iter()
                .map(|summary| &summary.id)
                .collect::<Vec<_>>(),
            vec![&first, &second]
        );
        assert_eq!(inspected.id, first);
        assert_eq!(unique.to_string(), "pen_00");
        assert_eq!(removed.id, first);
        assert_eq!(removed.removed_bytes, inspected.retained_bytes);
        assert_eq!(
            store
                .pending()
                .expect("the remaining pending capture can be listed")
                .into_iter()
                .map(|summary| summary.id)
                .collect::<Vec<_>>(),
            vec![second]
        );
    }

    #[test]
    fn rejects_ambiguous_unknown_and_read_only_pending_removal() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        write_pending_capture(
            &store,
            &"0".repeat(64),
            "cap_00000000000000000000000000000000",
        );
        let retained = write_pending_capture(
            &store,
            &"1".repeat(64),
            "cap_01000000000000000000000000000000",
        );

        assert!(matches!(
            store.pending_summary(&"pen_0".parse().expect("the pending prefix is valid")),
            Err(Error::AmbiguousIdentifier {
                kind: "pending capture",
                ..
            })
        ));
        assert!(matches!(
            store.pending_summary(&"pen_f".parse().expect("the pending prefix is valid")),
            Err(Error::UnknownPending { .. })
        ));

        drop(store);
        let optic_dir = temporary.path().join(".optic");
        let read_only = Store::open_read_only(&optic_dir).expect("the store can open read-only");

        assert_eq!(
            read_only
                .pending_summary(&retained)
                .expect("pending inspection is read-only")
                .id,
            retained
        );
        assert!(matches!(
            read_only.remove_pending(&retained),
            Err(Error::ReadOnlyStore {
                operation: "remove pending evidence",
                path,
            }) if path == optic_dir
        ));
    }

    #[test]
    fn shared_operation_lock_blocks_an_exclusive_lock() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let shared = lock_workspace_shared(temporary.path()).expect("the shared lock succeeds");
        let path = temporary.path().join(".optic/locks/operation.lock");
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("the test can open the operation lock");

        let error = contender
            .try_lock_exclusive()
            .expect_err("the shared lock blocks an exclusive lock");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);

        drop(shared);
        let _exclusive =
            lock_workspace_exclusive(temporary.path()).expect("the exclusive lock now succeeds");
    }

    #[cfg(unix)]
    #[test]
    fn operation_lock_rejects_a_symbolic_linked_optic_root() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let linked = temporary.path().join("linked");
        fs::create_dir(&linked).expect("the test can create the linked directory");
        symlink(&linked, temporary.path().join(".optic"))
            .expect("the test can create the symbolic link");

        assert!(matches!(
            lock_workspace_shared(temporary.path()),
            Err(Error::Filesystem { .. })
        ));
        assert!(!linked.join("locks").exists());
    }

    #[test]
    fn rejects_evidence_in_the_legacy_root_layout() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let legacy_catalog = temporary.path().join(".optic/catalog.sqlite");
        fs::create_dir_all(legacy_catalog.parent().expect("the catalog has a parent"))
            .expect("the test can create the legacy store");
        fs::write(&legacy_catalog, b"legacy").expect("the test can create a legacy catalog");

        assert!(matches!(
            Store::open(temporary.path()),
            Err(Error::LegacyStore { path }) if path == legacy_catalog
        ));
        assert!(!temporary.path().join(".optic/store").exists());
    }

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
                stage: Some(crate::LlvmStage::Optimized),
                selected: true,
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
                stage: Some(crate::LlvmStage::Optimized),
                selected: true,
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
    fn selects_the_final_thin_lto_artifact_as_optimized_output() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        store
            .connection
            .execute_batch(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES (
                     'cap_00000000000000000000000000000000', 0, 'key', '{}', '', '', '', '',
                     '', '', '', '', 'faithful', '{}'
                 );
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
    fn materialized_availability_uses_only_the_selected_thin_lto_body() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let mut store = lookup_store(temporary.path());
        let capture_id = lookup_capture_id();
        let body = BodyRange {
            raw_symbol: "_Rthin".to_owned(),
            demangled: "crate_a::thin".to_owned(),
            start: 0,
            end: 1,
        };
        let modules = [
            published_module("rcgu", "rcgu", false, body.clone()),
            published_module("thin", "thin-lto-after-pm", true, body),
        ];
        let instance = CompilerInstance {
            origin: DefinitionOrigin {
                crate_name: "crate_a".to_owned(),
                definition_path: "crate_a::thin".to_owned(),
                source: None,
            },
            display_name: "crate_a::thin".to_owned(),
            raw_symbol: "_Rthin".to_owned(),
            placements: Vec::new(),
        };
        let transaction = store
            .connection
            .transaction()
            .expect("the test can start a catalog transaction");
        let index = insert_modules(&transaction, &capture_id, &modules)
            .expect("the test can index the duplicate compiler stages");
        insert_instances(&transaction, &capture_id, &[instance], &index)
            .expect("the test can materialize instance availability");
        transaction
            .commit()
            .expect("the test can commit materialized availability");

        let result = store
            .find(&capture_id, &FindOptions::new("crate_a::thin"))
            .expect("the exact lookup succeeds");
        let optimized = &result.instances[0].availability[0];

        assert_eq!(optimized.definitions, 1);
    }

    #[test]
    fn streamed_relationships_use_only_the_instance_capture() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let mut store = lookup_store(temporary.path());
        let capture_id = "cap_22222222222222222222222222222222"
            .parse::<CaptureId>()
            .expect("the second capture ID is valid");
        store
            .connection
            .execute(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES (?1, 1, 'second', '{}', '', '', '', '', '', '', '', '', 'faithful', '{}')",
                [capture_id.as_str()],
            )
            .expect("the test can insert the second capture");
        store
            .connection
            .execute_batch(
                "INSERT INTO definitions(id, capture_id, crate_name, path) VALUES
                     ('def_shared', 'cap_22222222222222222222222222222222', 'crate', 'crate::f');
                 INSERT INTO instances(id, capture_id, definition_id, display_name, compiler_symbol)
                 VALUES (
                     'ins_22222222222222222222222222222222',
                     'cap_22222222222222222222222222222222',
                     'def_shared', 'crate::f', '_Rshared'
                 );
                 INSERT INTO modules(
                     id, capture_id, name, stage, compiler_stage, lto, capture_method,
                     bitcode_blob, text_blob
                 ) VALUES
                     ('old_module', 'cap_11111111111111111111111111111111', 'old',
                      'llvm-optimized', 'rcgu', 'none', 'saved-temporary', 'old_bc', 'old_ll'),
                     ('new_module', 'cap_22222222222222222222222222222222', 'new',
                      'llvm-optimized', 'rcgu', 'none', 'saved-temporary', 'new_bc', 'new_ll');
                 INSERT INTO bodies(id, module_id, symbol, start, end) VALUES
                     ('old_body', 'old_module', '_Rshared', 0, 1),
                     ('old_target', 'old_module', '_Rtarget', 1, 2),
                     ('new_body', 'new_module', '_Rshared', 0, 1),
                     ('new_target', 'new_module', '_Rtarget', 1, 2);
                 INSERT INTO declarations(id, module_id, symbol, start, end) VALUES
                     ('old_declaration', 'old_module', '_Rshared', 0, 1),
                     ('new_declaration', 'new_module', '_Rshared', 0, 1);
                 INSERT INTO aliases(
                     id, module_id, symbol, target_kind, target_symbol, start, end
                 ) VALUES
                     ('old_alias', 'old_module', '_Rshared', 'symbol', '_Rtarget', 0, 1),
                     ('new_alias', 'new_module', '_Rshared', 'symbol', '_Rtarget', 0, 1);",
            )
            .expect("the test can insert matching evidence in both captures");
        let transaction = store
            .connection
            .transaction()
            .expect("the test can start the relationship transaction");

        associate_streamed_bodies(&transaction, &capture_id)
            .expect("the test can associate streamed bodies");
        update_streamed_availability(&transaction, &capture_id)
            .expect("the test can update streamed availability");
        transaction
            .commit()
            .expect("the test can commit streamed relationships");

        let stale_bodies = store
            .connection
            .query_row(
                "SELECT count(*)
                 FROM instance_bodies
                 JOIN bodies ON bodies.id = instance_bodies.body_id
                 JOIN modules ON modules.id = bodies.module_id
                 WHERE instance_bodies.instance_id = 'ins_22222222222222222222222222222222'
                   AND modules.capture_id != 'cap_22222222222222222222222222222222'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("the test can count stale body associations");
        let availability = store
            .connection
            .query_row(
                "SELECT llvm_definitions, llvm_declarations, llvm_aliases
                 FROM instances WHERE id = 'ins_22222222222222222222222222222222'",
                [],
                |row| Ok([row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?]),
            )
            .expect("the test can read capture-scoped availability");

        assert_eq!(stale_bodies, 0);
        assert_eq!(availability, [2, 1, 1]);
    }

    #[test]
    fn stores_filters_and_reclaims_exact_symbol_remarks() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let mut store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "remark-definition",
            "remark_crate",
            "remark_crate::kernel",
            "remark_crate::kernel",
            "_Rremark_kernel",
            [1, 0, 0, 0, 0, 0],
        );
        insert_search_instance(
            &store,
            2,
            "second-remark-definition",
            "remark_crate",
            "remark_crate::second_kernel",
            "remark_crate::second_kernel",
            "_Rremark_kernel",
            [1, 0, 0, 0, 0, 0],
        );
        let instance_id = "ins_00000000000000000000000000000001"
            .parse::<InstanceId>()
            .expect("the test instance ID is valid");

        let missing = store
            .show_remarks(&instance_id, &RemarkOptions::default())
            .expect("the non-remark capture can be inspected");
        assert_eq!(missing.summary.state, RemarkEvidenceState::NotCaptured);
        store
            .connection
            .execute(
                "UPDATE captures SET remarks_captured = 1
                 WHERE id = 'cap_11111111111111111111111111111111'",
                [],
            )
            .expect("the test can represent an empty remark capture");
        let empty = store
            .show_remarks(&instance_id, &RemarkOptions::default())
            .expect("the empty remark capture can be inspected");
        assert_eq!(empty.summary.state, RemarkEvidenceState::CapturedEmpty);

        let raw_path = temporary.path().join("remarks.opt.opt.yaml");
        fs::write(&raw_path, b"raw remark evidence")
            .expect("the test can create raw remark evidence");
        let raw_blob = store
            .publish_blob(&raw_path)
            .expect("the raw remark can be published");
        store
            .connection
            .execute(
                "UPDATE captures SET
                     remark_file_count = 1, remark_record_count = 3,
                     remark_linked_record_count = 2
                 WHERE id = 'cap_11111111111111111111111111111111'",
                [],
            )
            .expect("the test can record remark summary counts");
        let records = vec![
            test_remark(
                cargo_ir::RemarkKind::Passed,
                "inline",
                "_Rremark_kernel",
                "inlined",
            ),
            test_remark(
                cargo_ir::RemarkKind::Missed,
                "loop-vectorize",
                "_Rremark_kernel",
                "not vectorized",
            ),
            test_remark(
                cargo_ir::RemarkKind::Analysis,
                "inline",
                "_Runlinked",
                "unlinked",
            ),
        ];
        let second_instance = "ins_00000000000000000000000000000002"
            .parse::<InstanceId>()
            .expect("the second test instance ID is valid");
        let instance_index = HashMap::from([(
            "_Rremark_kernel".to_owned(),
            vec![instance_id.clone(), second_instance.clone()],
        )]);
        let transaction = store
            .connection
            .transaction()
            .expect("the test can start a remark transaction");
        insert_remarks(
            &transaction,
            &lookup_capture_id(),
            &[PublishedRemarkFile {
                name: "crate.opt.opt.yaml".to_owned(),
                blob: raw_blob,
                records,
            }],
            &instance_index,
        )
        .expect("the test can insert optimization remarks");
        transaction
            .commit()
            .expect("the test can commit optimization remarks");

        let remarks = store
            .show_remarks(&instance_id, &RemarkOptions::default())
            .expect("the exact-symbol remarks can be shown");
        assert_eq!(remarks.summary.state, RemarkEvidenceState::Captured);
        assert_eq!(remarks.summary.records, 3);
        assert_eq!(remarks.summary.linked_records, 2);
        assert_eq!(remarks.summary.unlinked_records, 1);
        assert_eq!(remarks.remarks.len(), 2);
        assert!(!remarks.truncated);
        let second_remarks = store
            .show_remarks(&second_instance, &RemarkOptions::default())
            .expect("the second exact-symbol instance has the same linked records");
        assert_eq!(second_remarks.remarks.len(), 2);

        let filtered = store
            .show_remarks(
                &instance_id,
                &RemarkOptions {
                    kind: Some(RemarkKindFilter::Passed),
                    pass: Some("inline".to_owned()),
                    limit: 1,
                },
            )
            .expect("remark filters can be applied");
        assert_eq!(filtered.remarks.len(), 1);
        assert_eq!(filtered.remarks[0].message, "inlined");

        let limited = store
            .show_remarks(
                &instance_id,
                &RemarkOptions {
                    limit: 1,
                    ..RemarkOptions::default()
                },
            )
            .expect("remark limits can be applied");
        assert!(limited.truncated);

        store
            .connection
            .execute(
                "UPDATE remarks SET arguments_json = 'not json' WHERE message = 'inlined'",
                [],
            )
            .expect("the test can corrupt typed remark arguments");
        assert!(
            store
                .show_remarks(&instance_id, &RemarkOptions::default())
                .is_err()
        );
        store
            .connection
            .execute(
                "UPDATE remarks SET arguments_json = '[]' WHERE message = 'inlined'",
                [],
            )
            .expect("the test can restore typed remark arguments");

        assert!(
            store
                .verify()
                .expect("raw remark verification succeeds")
                .verified_blobs
                > 0
        );
        store
            .remove_capture(&lookup_capture_id())
            .expect("the remark capture can be removed");
        assert!(
            store
                .gc()
                .expect("the raw remark blob can be reclaimed")
                .removed_blobs
                > 0
        );
    }

    #[test]
    fn exact_lookup_accepts_a_short_query_and_exposes_symbol_identity() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "left",
            "crate_a",
            "x",
            "x::<u64>",
            "_Rexact",
            [1, 0, 0, 0, 0, 0],
        );
        let capture_id = lookup_capture_id();

        let result = store
            .find(&capture_id, &FindOptions::new("x"))
            .expect("the exact lookup succeeds");

        assert_eq!(result.match_kind, FindMatchKind::Exact);
        assert!(!result.truncated);
        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.instances[0].compiler_symbol, "_Rexact");
        assert_eq!(
            result.instances[0].symbol_fingerprint,
            &blake3::hash(b"_Rexact").to_hex()[..12]
        );
    }

    fn test_remark(
        kind: cargo_ir::RemarkKind,
        pass_name: &str,
        function: &str,
        message: &str,
    ) -> cargo_ir::OptimizationRemark {
        cargo_ir::OptimizationRemark {
            kind,
            pass_name: pass_name.to_owned(),
            remark_name: "TestRemark".to_owned(),
            function: function.to_owned(),
            source_location: None,
            hotness: None,
            arguments: Vec::new(),
            message: message.to_owned(),
        }
    }

    #[test]
    fn short_substring_lookup_is_rejected_after_an_exact_miss() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "short",
            "crate_a",
            "crate_a::kernel",
            "crate_a::kernel",
            "_Rshort",
            [1, 0, 0, 0, 0, 0],
        );

        assert!(matches!(
            store.find(&lookup_capture_id(), &FindOptions::new("ke")),
            Err(Error::InvalidRequest { .. })
        ));
    }

    #[test]
    fn substring_lookup_is_case_sensitive_unicode_and_literal() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "unicode",
            "crate_a",
            "crate_a::Café[slot]",
            "crate_a::Café[slot]::<u64>",
            "_Runicode",
            [1, 0, 0, 0, 0, 0],
        );

        let literal = store
            .find(&lookup_capture_id(), &FindOptions::new("fé[slot]"))
            .expect("FTS treats punctuation as literal text");
        let different_case = store
            .find(&lookup_capture_id(), &FindOptions::new("CAFÉ"))
            .expect("a different case is a valid lookup");

        assert_eq!(literal.match_kind, FindMatchKind::Substring);
        assert_eq!(literal.instances.len(), 1);
        assert!(different_case.instances.is_empty());
    }

    #[test]
    fn substring_lookup_quotes_fts_operators_and_double_quotes() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "operators",
            "crate_a",
            "crate_a::kernel_\"AND OR NOT\"",
            "crate_a::kernel_\"AND OR NOT\"::<u64>",
            "_Roperators",
            [1, 0, 0, 0, 0, 0],
        );

        let result = store
            .find(&lookup_capture_id(), &FindOptions::new("\"AND OR NOT\""))
            .expect("FTS operators and quotes are literal text");

        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.match_kind, FindMatchKind::Substring);
    }

    #[test]
    fn lookup_filters_availability_and_reports_truncation() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "first",
            "crate_a",
            "crate_a::common_first",
            "common::<u32>",
            "_Rfirst",
            [1, 0, 0, 0, 0, 0],
        );
        insert_search_instance(
            &store,
            2,
            "second",
            "crate_b",
            "crate_b::common_second",
            "common::<u64>",
            "_Rsecond",
            [0, 0, 0, 1, 0, 0],
        );
        let mut options = FindOptions::new("common");
        options.crate_name = Some("crate_b".to_owned());
        options.definition = Some("crate_b::common_second".to_owned());
        options.available = Some(CompilerOutput::LlvmPreOpt);
        options.limit = 1;

        let filtered = store
            .find(&lookup_capture_id(), &options)
            .expect("the filtered lookup succeeds");
        options.crate_name = None;
        options.definition = None;
        options.available = None;
        let truncated = store
            .find(&lookup_capture_id(), &options)
            .expect("the limited lookup succeeds");

        assert_eq!(filtered.instances.len(), 1);
        assert_eq!(filtered.instances[0].crate_name, "crate_b");
        assert!(!filtered.truncated);
        assert_eq!(truncated.instances.len(), 1);
        assert!(truncated.truncated);
    }

    #[test]
    fn capture_removal_cleans_the_explicit_search_index() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let mut store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "remove",
            "crate_a",
            "crate_a::remove_kernel",
            "crate_a::remove_kernel",
            "_Rremove",
            [1, 0, 0, 0, 0, 0],
        );

        store
            .remove_capture(&lookup_capture_id())
            .expect("capture removal succeeds");
        let rows = store
            .connection
            .query_row("SELECT COUNT(*) FROM instance_search", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("the search index remains readable");

        assert_eq!(rows, 0);
        store.verify().expect("the empty search index verifies");
    }

    #[test]
    fn verification_rejects_stale_search_text() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        insert_search_instance(
            &store,
            1,
            "stale",
            "crate_a",
            "crate_a::kernel",
            "crate_a::kernel",
            "_Rstale",
            [1, 0, 0, 0, 0, 0],
        );
        store
            .connection
            .execute("UPDATE instance_search SET display_name = 'corrupt'", [])
            .expect("the test can corrupt the search index");

        assert!(matches!(
            store.verify(),
            Err(Error::InvalidStoredData { .. })
        ));
    }

    #[test]
    #[ignore = "creates a synthetic 100,000-instance lookup catalog"]
    fn broad_lookup_stays_below_one_second_after_warmup() {
        use std::time::{Duration, Instant};

        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = lookup_store(temporary.path());
        store
            .connection
            .execute_batch(
                "BEGIN;
                 WITH RECURSIVE numbers(value) AS (
                     VALUES(1)
                     UNION ALL
                     SELECT value + 1 FROM numbers WHERE value < 100000
                 )
                 INSERT INTO definitions(id, capture_id, crate_name, path)
                 SELECT 'def_' || printf('%032x', value),
                        'cap_11111111111111111111111111111111',
                        'synthetic',
                        'synthetic::synthetic_kernel_' || printf('%06d', value)
                 FROM numbers;
                 INSERT INTO instances(
                     id, capture_id, definition_id, display_name, compiler_symbol,
                     llvm_definitions
                 )
                 SELECT 'ins_' || substr(definitions.id, 5),
                        definitions.capture_id,
                        definitions.id,
                        definitions.path,
                        '_Rsynthetic_' || substr(definitions.id, 5),
                        1
                 FROM definitions;
                 INSERT INTO instance_search(
                     rowid, instance_id, capture_id, definition_path, display_name,
                     compiler_symbol
                 )
                 SELECT instances.rowid, instances.id, instances.capture_id, definitions.path,
                        instances.display_name, instances.compiler_symbol
                 FROM instances JOIN definitions ON definitions.id = instances.definition_id;
                 COMMIT;",
            )
            .expect("the test can create the synthetic lookup catalog");
        let options = FindOptions::new("synthetic_kernel");
        let run = || {
            let start = Instant::now();
            let result = store
                .find(&lookup_capture_id(), &options)
                .expect("the synthetic lookup succeeds");
            assert_eq!(result.instances.len(), FindOptions::DEFAULT_LIMIT);
            assert!(result.truncated);

            start.elapsed()
        };
        let _warmup = run();
        let samples = (0..5).map(|_| run()).collect::<Vec<_>>();

        eprintln!("100k lookup samples: {samples:?}");
        assert!(
            samples
                .iter()
                .all(|sample| *sample < Duration::from_secs(1))
        );
    }

    #[test]
    fn publishes_and_reuses_a_verified_blob() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        let contents = "define void @kernel() {}\n".repeat(1_000);
        fs::write(&source, &contents).expect("the test can write source evidence");

        let digest = store
            .publish_blob(&source)
            .expect("the store can publish the source evidence");
        let reused_digest = store
            .publish_blob(&source)
            .expect("the store can verify and reuse the published evidence");
        let destination = store.blob_path(&digest);

        assert_eq!(digest, blake3::hash(contents.as_bytes()).to_hex().as_str());
        assert_eq!(reused_digest, digest);
        assert_eq!(
            store
                .read_blob(&digest)
                .expect("the store can decompress the published evidence"),
            contents.as_bytes()
        );
        assert!(
            fs::metadata(destination)
                .expect("the compressed blob has metadata")
                .len()
                < contents.len() as u64
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
        assert_invalid_data(store.read_blob_range(&digest, 0, 8));
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
    fn streams_verified_utf8_ranges_in_bounded_chunks() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        let contents = format!("{}é{}", "a".repeat(crate::TEXT_CHUNK_BYTES), "b".repeat(32));
        fs::write(&source, &contents).expect("the test can write UTF-8 evidence");
        let digest = store
            .publish_blob(&source)
            .expect("the store can publish the source evidence");
        let mut chunks = Vec::new();

        store
            .read_blob_range_with(&digest, 0, contents.len() as i64, |chunk| {
                chunks.push(chunk);

                Ok(())
            })
            .expect("the store can stream the complete range");

        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= crate::TEXT_CHUNK_BYTES)
        );
        assert_eq!(chunks.concat(), contents);
    }

    #[test]
    fn reads_a_nonzero_logical_range_from_a_compressed_blob() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let source = temporary.path().join("source.ll");
        let contents = "prefix\ndefine i32 @kernel() { ret i32 42 }\nsuffix\n";
        fs::write(&source, contents).expect("the test can write source evidence");
        let digest = store
            .publish_blob(&source)
            .expect("the store can publish source evidence");
        let start = contents.find("define").expect("the body has a start") as i64;
        let end = contents.find("\nsuffix").expect("the body has an end") as i64;

        assert_eq!(
            store
                .read_blob_range(&digest, start, end)
                .expect("the store can read a logical byte range"),
            "define i32 @kernel() { ret i32 42 }"
        );
    }

    #[test]
    fn rejects_truncated_and_bit_flipped_compressed_blobs() {
        for corrupt in ["truncate", "bit flip"] {
            let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
            let store = Store::open(temporary.path()).expect("the test can open a store");
            let source = temporary.path().join("source.ll");
            fs::write(&source, "define void @kernel() {}\n".repeat(1_000))
                .expect("the test can write source evidence");
            let digest = store
                .publish_blob(&source)
                .expect("the store can publish source evidence");
            let path = store.blob_path(&digest);

            if corrupt == "truncate" {
                let file = OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("the test can open the blob");
                let length = file
                    .metadata()
                    .expect("the compressed blob has metadata")
                    .len();
                file.set_len(length - 1)
                    .expect("the test can truncate the blob");
            } else {
                let mut bytes = fs::read(&path).expect("the test can read the blob");
                let middle = bytes.len() / 2;
                bytes[middle] ^= 0x80;
                fs::write(&path, bytes).expect("the test can corrupt the blob");
            }

            assert_invalid_data(store.read_blob(&digest));
        }
    }

    #[test]
    fn rejects_a_schema_five_store() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let catalog = temporary.path().join("catalog.sqlite");
        let connection = Connection::open(&catalog).expect("the test can open the catalog");
        connection
            .pragma_update(None, "user_version", 5)
            .expect("the test can create a schema-five catalog");
        drop(connection);

        let store = temporary.path().join(".optic/store");
        fs::create_dir_all(&store).expect("the test can create the store directory");
        fs::rename(catalog, store.join("catalog.sqlite"))
            .expect("the test can install the schema-five catalog");

        assert!(matches!(
            Store::open(temporary.path()),
            Err(Error::StoreVersion {
                expected: STORE_VERSION,
                actual: 5,
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
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES
                     ('cap_00000000000000000000000000000000', 0, 'a', '{}', '', '', '', '', '',
                      '', '', '', 'faithful', '{}'),
                     ('cap_0fffffffffffffffffffffffffffffff', 0, 'b', '{}', '', '', '', '', '',
                      '', '', '', 'faithful', '{}');
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
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES (
                     'invalid', 0, 'request', '{}', '', '', '', '', '', '', '', '', '', '{}'
                 )",
                [],
            )
            .expect("the test can insert an invalid stored identifier");

        assert!(matches!(store.captures(), Err(Error::Database(_))));
    }

    #[test]
    fn rejects_an_invalid_stored_analysis_key() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        store
            .connection
            .execute_batch(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES (
                     'cap_00000000000000000000000000000000', 0, 'request', '{}', '', '', '', '',
                     '', '', '', '', 'faithful', '{}'
                 );
                 INSERT INTO capture_cache(request_key, capture_id, analysis_key) VALUES (
                     'request', 'cap_00000000000000000000000000000000', '../outside'
                 );",
            )
            .expect("the test can insert an invalid analysis key");

        assert!(matches!(
            store.cached_capture("request"),
            Err(Error::Database(_))
        ));
    }

    #[test]
    fn recognizes_a_committed_pending_capture_before_reingestion() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let store = Store::open(temporary.path()).expect("the test can open a store");
        let capture_id = "cap_00000000000000000000000000000000"
            .parse::<CaptureId>()
            .expect("the test capture ID is valid");
        let analysis_key =
            super::AnalysisKey::parse(&"0".repeat(32)).expect("the test analysis key is valid");
        store
            .connection
            .execute_batch(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES (
                     'cap_00000000000000000000000000000000', 0, 'request', '{}', '', '', '', '',
                     '', '', '', '', 'faithful', '{}'
                 );
                 INSERT INTO capture_cache(request_key, capture_id, analysis_key) VALUES (
                     'request', 'cap_00000000000000000000000000000000',
                     '00000000000000000000000000000000'
                 );",
            )
            .expect("the test can record a committed capture");

        let summary = store
            .completed_capture(
                &capture_id,
                "request",
                &analysis_key,
                CaptureDisposition::Resumed,
            )
            .expect("the committed capture can be checked")
            .expect("the committed capture is present");

        assert_eq!(summary.id, capture_id);
        assert_eq!(summary.disposition, CaptureDisposition::Resumed);
    }

    fn lookup_store(path: &std::path::Path) -> Store {
        let store = Store::open(path).expect("the test can open a lookup store");
        store
            .connection
            .execute(
                "INSERT INTO captures(
                     id, created_at_ms, request_key, request_json, rustc_path, rustc_release,
                     rustc_commit, rustc_host, llvm_version, rustc_sysroot, llvm_dis_path, target,
                     profile, invocation_json
                 ) VALUES (
                     ?1, 0, 'lookup', '{}', '', '', '', '', '', '', '', '', 'faithful', '{}'
                 )",
                [lookup_capture_id().as_str()],
            )
            .expect("the test can insert the lookup capture");

        store
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_search_instance(
        store: &Store,
        rowid: i64,
        id_suffix: &str,
        crate_name: &str,
        definition: &str,
        display_name: &str,
        compiler_symbol: &str,
        availability: [i64; 6],
    ) {
        let definition_id = format!("def_{id_suffix}");
        let instance_id = format!("ins_{rowid:032x}");
        store
            .connection
            .execute(
                "INSERT INTO definitions(id, capture_id, crate_name, path)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    definition_id,
                    lookup_capture_id().as_str(),
                    crate_name,
                    definition,
                ],
            )
            .expect("the test can insert a searchable definition");
        store
            .connection
            .execute(
                "INSERT INTO instances(
                     id, capture_id, definition_id, display_name, compiler_symbol,
                     llvm_definitions, llvm_declarations, llvm_aliases,
                     pre_opt_definitions, pre_opt_declarations, pre_opt_aliases
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    instance_id,
                    lookup_capture_id().as_str(),
                    definition_id,
                    display_name,
                    compiler_symbol,
                    availability[0],
                    availability[1],
                    availability[2],
                    availability[3],
                    availability[4],
                    availability[5],
                ],
            )
            .expect("the test can insert a searchable instance");
        let stored_rowid = store.connection.last_insert_rowid();
        store
            .connection
            .execute(
                "INSERT INTO instance_search(
                     rowid, instance_id, capture_id, definition_path, display_name,
                     compiler_symbol
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    stored_rowid,
                    instance_id,
                    lookup_capture_id().as_str(),
                    definition,
                    display_name,
                    compiler_symbol,
                ],
            )
            .expect("the test can index a searchable instance");
    }

    fn lookup_capture_id() -> CaptureId {
        "cap_11111111111111111111111111111111"
            .parse()
            .expect("the lookup capture ID is valid")
    }

    fn published_module(
        name: &str,
        compiler_stage: &str,
        selected: bool,
        body: BodyRange,
    ) -> PublishedModule {
        PublishedModule {
            name: name.to_owned(),
            provenance: ArtifactProvenance {
                stage: Some(crate::LlvmStage::Optimized),
                compiler_stage: compiler_stage.to_owned(),
                codegen_unit: Some("crate.cgu.0".to_owned()),
                lto: if selected {
                    LtoScope::Thin
                } else {
                    LtoScope::None
                },
                capture_method: CaptureMethod::SavedTemporary,
            },
            bitcode_blob: format!("{name}-bc"),
            text_blob: format!("{name}-ll"),
            bodies: vec![body],
            declarations: Vec::new(),
            aliases: Vec::new(),
            selected,
        }
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
