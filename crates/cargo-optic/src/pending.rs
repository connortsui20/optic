//! Retains completed compiler runs until their evidence is published.
//!
//! A request-key directory owns its compiler artifacts, source snapshots, and marker. The marker
//! is written last. Its opaque keys select every mutable path, so persisted JSON cannot redirect
//! ingestion to another directory.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::source::{PendingSourceBaseline, SourceBaseline};
use crate::store::{AnalysisKey, sync_directory};
use crate::{BuildSpec, CaptureId, Error, Result};

const PENDING_VERSION: u32 = 1;
const MARKER_NAME: &str = "pending.json";
const MAX_MARKER_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PendingCapture {
    version: u32,

    request_key: String,

    capture_id: CaptureId,

    analysis_key: String,

    spec: BuildSpec,

    compilation: cargo_ir::CompiledCapture,

    sources: PendingSourceBaseline,
}

pub(crate) struct ResumableCapture {
    pub(crate) capture_id: CaptureId,

    pub(crate) analysis_key: AnalysisKey,

    pub(crate) compilation: cargo_ir::CompiledCapture,

    pub(crate) sources: SourceBaseline,
}

impl PendingCapture {
    pub(crate) fn new(
        request_key: &str,
        capture_id: CaptureId,
        analysis_key: &AnalysisKey,
        spec: BuildSpec,
        compilation: cargo_ir::CompiledCapture,
        sources: PendingSourceBaseline,
    ) -> Self {
        Self {
            version: PENDING_VERSION,
            request_key: request_key.to_owned(),
            capture_id,
            analysis_key: analysis_key.as_str().to_owned(),
            spec,
            compilation,
            sources,
        }
    }

    pub(crate) fn marker_path(directory: &Path) -> PathBuf {
        directory.join(MARKER_NAME)
    }

    pub(crate) fn exists(directory: &Path) -> bool {
        Self::marker_path(directory).is_file()
    }

    pub(crate) fn write(&self, directory: &Path) -> Result<()> {
        sync_payload(directory)?;
        let marker = Self::marker_path(directory);
        let temporary = directory.join(format!(".{MARKER_NAME}.tmp"));
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_MARKER_BYTES {
            return Err(Error::InvalidPendingEvidence {
                path: marker,
                message: format!(
                    "pending marker length must be at most {MAX_MARKER_BYTES} bytes, got {}",
                    bytes.len()
                ),
            });
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| Error::filesystem("create", &temporary, source))?;
        file.write_all(&bytes)
            .map_err(|source| Error::filesystem("write", &temporary, source))?;
        file.sync_all()
            .map_err(|source| Error::filesystem("synchronize", &temporary, source))?;
        fs::rename(&temporary, &marker)
            .map_err(|source| Error::filesystem("publish", &marker, source))?;
        sync_directory(directory)?;

        Ok(())
    }

    pub(crate) fn resume(
        directory: &Path,
        request_key: &str,
        spec: &BuildSpec,
        request_template: &cargo_ir::BuildRequest,
        toolchain: &cargo_ir::Toolchain,
        check_cargo_freshness: bool,
    ) -> Result<ResumableCapture> {
        let marker = Self::marker_path(directory);
        let file =
            File::open(&marker).map_err(|source| Error::filesystem("open", &marker, source))?;
        let length = file
            .metadata()
            .map_err(|source| Error::filesystem("read metadata for", &marker, source))?
            .len();
        if length > MAX_MARKER_BYTES {
            return Err(Error::InvalidPendingEvidence {
                path: marker,
                message: format!(
                    "pending marker length must be at most {MAX_MARKER_BYTES} bytes, got {length}"
                ),
            });
        }
        let pending = serde_json::from_reader::<_, Self>(BufReader::new(
            file.take(MAX_MARKER_BYTES.saturating_add(1)),
        ))
        .map_err(|source| Error::InvalidPendingEvidence {
            path: marker.clone(),
            message: source.to_string(),
        })?;

        let analysis_key =
            AnalysisKey::parse(&pending.analysis_key).map_err(|error| match error {
                Error::InvalidPendingEvidence { message, .. } => Error::InvalidPendingEvidence {
                    path: marker.clone(),
                    message,
                },
                error => error,
            })?;
        let mut request = request_template.clone();
        request.analysis_directory = directory.join(analysis_key.as_str()).join("analysis");
        pending.validate(&marker, request_key, spec, &request, toolchain)?;
        if check_cargo_freshness && !cargo_ir::check_fresh(&request, toolchain)? {
            return Err(Error::PendingInputsChanged);
        }
        let staging = directory.join(analysis_key.as_str()).join("staging");
        let sources = SourceBaseline::resume(
            &request.workspace_root,
            spec,
            &staging,
            &pending.sources,
            &marker,
        )?;

        Ok(ResumableCapture {
            capture_id: pending.capture_id,
            analysis_key,
            compilation: pending.compilation,
            sources,
        })
    }

    fn validate(
        &self,
        marker: &Path,
        request_key: &str,
        spec: &BuildSpec,
        request: &cargo_ir::BuildRequest,
        toolchain: &cargo_ir::Toolchain,
    ) -> Result<()> {
        if self.version != PENDING_VERSION {
            return Err(invalid(
                marker,
                format!(
                    "pending format must be {PENDING_VERSION}, got {}",
                    self.version
                ),
            ));
        }
        if self.request_key != request_key {
            return Err(invalid(
                marker,
                format!(
                    "request key must match its directory, got {}",
                    self.request_key
                ),
            ));
        }
        if self.capture_id.as_str().len() != 36 {
            return Err(invalid(
                marker,
                format!("capture ID must be complete, got {}", self.capture_id),
            ));
        }
        if &self.spec != spec {
            return Err(invalid(
                marker,
                "build request does not match the current request",
            ));
        }
        if self.compilation.toolchain != *toolchain {
            return Err(invalid(
                marker,
                "compiler identity does not match the workspace compiler",
            ));
        }
        if self.compilation.invocation.request != *request {
            return Err(invalid(
                marker,
                "compiler request does not match its derived artifact path",
            ));
        }

        cargo_ir::require_compiled_evidence(request).map_err(|error| {
            invalid(
                marker,
                format!("compiler artifacts are incomplete: {error}"),
            )
        })
    }
}

fn sync_payload(root: &Path) -> Result<()> {
    let mut directories = Vec::new();

    for entry in WalkDir::new(root) {
        let entry = entry.map_err(|source| Error::filesystem("walk", root, source.into()))?;
        if entry.file_type().is_dir() {
            directories.push(entry.path().to_owned());
        } else if entry.file_type().is_file() {
            File::open(entry.path())
                .map_err(|source| Error::filesystem("open", entry.path(), source))?
                .sync_all()
                .map_err(|source| Error::filesystem("synchronize", entry.path(), source))?;
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }

    Ok(())
}

fn invalid(path: &Path, message: impl Into<String>) -> Error {
    Error::InvalidPendingEvidence {
        path: path.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{MAX_MARKER_BYTES, PendingCapture};
    use crate::{BuildSpec, Error};

    #[test]
    fn malformed_marker_returns_an_error_and_remains_recoverable() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let marker = PendingCapture::marker_path(temporary.path());
        fs::write(&marker, b"{").expect("the test can create a malformed marker");
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
            analysis_directory: PathBuf::new(),
        };
        let toolchain = cargo_ir::Toolchain {
            rustc: PathBuf::from("rustc"),
            release: "test".to_owned(),
            commit_hash: "0".repeat(40),
            host: "test-host".to_owned(),
            llvm_version: "test".to_owned(),
            sysroot: PathBuf::from("sysroot"),
            rustc_private_lib: PathBuf::from("rustc-private"),
            llvm_dis: PathBuf::from("llvm-dis"),
            rustup_toolchain: None,
        };

        let result = PendingCapture::resume(
            temporary.path(),
            &"0".repeat(64),
            &BuildSpec::default(),
            &request,
            &toolchain,
            false,
        );

        assert!(matches!(result, Err(Error::InvalidPendingEvidence { .. })));
        assert!(marker.is_file());
    }

    #[test]
    fn oversized_marker_returns_an_error_and_remains_recoverable() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let marker = PendingCapture::marker_path(temporary.path());
        let file = fs::File::create(&marker).expect("the test can create an oversized marker");
        file.set_len(MAX_MARKER_BYTES + 1)
            .expect("the test can size the marker");
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
            analysis_directory: PathBuf::new(),
        };
        let toolchain = cargo_ir::Toolchain {
            rustc: PathBuf::from("rustc"),
            release: "test".to_owned(),
            commit_hash: "0".repeat(40),
            host: "test-host".to_owned(),
            llvm_version: "test".to_owned(),
            sysroot: PathBuf::from("sysroot"),
            rustc_private_lib: PathBuf::from("rustc-private"),
            llvm_dis: PathBuf::from("llvm-dis"),
            rustup_toolchain: None,
        };

        let result = PendingCapture::resume(
            temporary.path(),
            &"0".repeat(64),
            &BuildSpec::default(),
            &request,
            &toolchain,
            false,
        );

        assert!(matches!(result, Err(Error::InvalidPendingEvidence { .. })));
        assert!(marker.is_file());
    }
}
