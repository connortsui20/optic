//! Selects source snapshots through their stored instance relationships.
//!
//! Display paths identify the original source for diagnostics. Queries never open those paths or
//! substitute current checkout contents for the compiler's captured text.

use std::path::PathBuf;

use optic_records::InstanceRef;
use optic_records::SourceAvailability;
use optic_records::SourceUnavailable;
use optic_store::Store;
use snafu::ResultExt;

use crate::Error;
use crate::EvidenceRange;
use crate::error;
use crate::referenced_instance;

/// Captured source for an instance, or the compiler's explicit unavailable reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceEvidence {
    /// The compiler retained a supported source span in a snapshot.
    Available {
        /// The bytes to copy from the completed capture.
        evidence: EvidenceRange,
        /// The original source path, used only as display metadata.
        display_path: PathBuf,
        /// The one-based source line at the start of the excerpt.
        starting_line: u64,
    },
    /// The compiler could not attribute supported source to this instance.
    Unavailable(SourceUnavailable),
}

/// Selects the captured source range for an immutable instance reference.
///
/// # Errors
///
/// Returns an error if the capture or its artifacts cannot be read or validated, or the ordinal is
/// outside the manifest. These failures do not become source unavailability.
pub fn source_evidence(store: &Store, reference: &InstanceRef) -> Result<SourceEvidence, Error> {
    let manifest = store
        .read_instances(reference.capture_id())
        .context(error::StoreSnafu)?;
    let instance = referenced_instance(&manifest, reference)?;

    match instance.source() {
        SourceAvailability::Available(source) => Ok(SourceEvidence::Available {
            evidence: EvidenceRange::new(
                reference.capture_id().clone(),
                source.artifact(),
                source.range(),
            ),
            display_path: source.display_path().to_owned(),
            starting_line: source.starting_line(),
        }),
        SourceAvailability::Unavailable(reason) => Ok(SourceEvidence::Unavailable(*reason)),
    }
}
