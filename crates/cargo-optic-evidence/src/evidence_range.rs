//! Identifies a finite excerpt within one completed capture.
//!
//! The descriptor retains capture scope through query composition. The store owns path resolution
//! and validates artifact bounds again when the caller copies the bytes.

use optic_records::ArtifactId;
use optic_records::ByteRange;
use optic_records::CaptureId;

/// A capture-scoped artifact range for [`optic_store::Store::copy_evidence`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRange {
    capture_id: CaptureId,
    artifact: ArtifactId,
    range: ByteRange,
}

impl EvidenceRange {
    /// Retains an artifact range from a validated manifest.
    ///
    /// The artifact and range **must** come from a manifest validated for `capture_id`. Combining
    /// fields from different manifests can select unrelated bytes when the store copies the range.
    pub(crate) fn new(capture_id: CaptureId, artifact: ArtifactId, range: ByteRange) -> Self {
        Self {
            capture_id,
            artifact,
            range,
        }
    }

    /// Returns the capture that owns the artifact.
    pub fn capture_id(&self) -> &CaptureId {
        &self.capture_id
    }

    /// Returns the artifact ID within this capture.
    pub fn artifact(&self) -> ArtifactId {
        self.artifact
    }

    /// Returns the half-open byte range within the artifact.
    pub fn range(&self) -> ByteRange {
        self.range
    }
}
