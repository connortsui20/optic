//! Resolves one request pointer to complete validated evidence.
//!
//! Only absent pointers and absent capture directories are misses. Present corrupt data returns an
//! error. Historical captures never restore a missing or dangling candidate.

use std::path::PathBuf;

use optic_records::AnalysisToken;
use optic_records::CaptureId;
use optic_records::CaptureKey;
use optic_records::CaptureRecord;
use optic_records::InstanceManifest;
use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::MAX_HEADER_BYTES;
use crate::MAX_INSTANCES_BYTES;
use crate::Store;
use crate::artifacts::validate_artifacts;
use crate::captures::read_capture_from_directory;
use crate::captures::validate_capture_scope;
use crate::error::InvalidCandidateSnafu;
use crate::record_io::read_record;

/// Revision 1 associates a request digest, compilation token, and completed capture ID.
/// The request digest includes the compiler's driver key and evidence policy revision.
const POINTER_FORMAT_VERSION: u32 = 1;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidatePointer {
    format_version: u32,
    request_key: CaptureKey,
    token: AnalysisToken,
    capture_id: CaptureId,
}

impl CandidatePointer {
    pub(crate) fn new(capture: &CaptureRecord) -> Self {
        Self {
            format_version: POINTER_FORMAT_VERSION,
            request_key: capture.analysis().request_key().clone(),
            token: capture.analysis().token().clone(),
            capture_id: capture.id().clone(),
        }
    }
}

impl Store {
    /// Returns the candidate and manifest after record and artifact validation.
    ///
    /// An absent pointer or referenced capture directory returns `None`. This operation does not
    /// change store state or establish Cargo freshness.
    /// The returned manifest retains stored LLVM unavailability without requiring another read.
    ///
    /// # Errors
    ///
    /// Returns an error for present malformed, oversized, incompatible, or inconsistent records,
    /// symlinks, wrong file types, and filesystem failures.
    pub fn read_candidate(
        &self,
        key: &CaptureKey,
    ) -> Result<Option<(CaptureRecord, InstanceManifest)>, Error> {
        if !self.namespace_exists("candidates")? {
            return Ok(None);
        }

        let path = self.candidate_path(key);
        let pointer: CandidatePointer = match read_record(&path, MAX_HEADER_BYTES) {
            Ok(pointer) => pointer,
            Err(Error::Filesystem { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if pointer.format_version != POINTER_FORMAT_VERSION {
            return InvalidCandidateSnafu {
                path,
                expected: format!("pointer format {POINTER_FORMAT_VERSION}"),
                actual: format!("pointer format {}", pointer.format_version),
            }
            .fail();
        }
        if &pointer.request_key != key {
            return InvalidCandidateSnafu {
                path,
                expected: key.to_string(),
                actual: pointer.request_key.to_string(),
            }
            .fail();
        }

        let directory = match self.capture_directory(&pointer.capture_id) {
            Ok(directory) => directory,
            Err(Error::CaptureNotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let capture = read_capture_from_directory(&directory, &pointer.capture_id)?;
        if capture.analysis().request_key() != key {
            return InvalidCandidateSnafu {
                path,
                expected: key.to_string(),
                actual: capture.analysis().request_key().to_string(),
            }
            .fail();
        }
        if capture.analysis().token() != &pointer.token {
            return InvalidCandidateSnafu {
                path,
                expected: pointer.token.to_string(),
                actual: capture.analysis().token().to_string(),
            }
            .fail();
        }

        let instances_path = directory.join(INSTANCES_FILE_NAME);
        let instances: InstanceManifest = read_record(&instances_path, MAX_INSTANCES_BYTES)?;
        validate_capture_scope(&instances_path, capture.id(), instances.capture_id())?;
        validate_artifacts(&directory, &instances)?;

        Ok(Some((capture, instances)))
    }

    pub(crate) fn candidate_path(&self, key: &CaptureKey) -> PathBuf {
        self.root
            .join("candidates")
            .join(format!("{}.json", key.as_str()))
    }
}
