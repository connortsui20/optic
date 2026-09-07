//! Searches concrete compiler instances and selects their stored source or LLVM evidence.
//!
//! Queries read only the explicitly selected capture. [`find_instances`] supplies immutable
//! references for [`source_evidence`] and [`llvm_evidence`]. The store validates durable records
//! and artifact files, then copies selected ranges with [`optic_store::Store::copy_evidence`].

use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::InstanceRef;

mod error;
pub use error::Error;

mod evidence_range;
pub use evidence_range::EvidenceRange;

mod find;
pub use find::FindResults;
pub use find::FoundInstance;
pub use find::MatchKind;
pub use find::find_instances;

mod source;
pub use source::SourceEvidence;
pub use source::source_evidence;

mod llvm;
pub use llvm::LlvmBody;
pub use llvm::LlvmEvidence;
pub use llvm::llvm_evidence;

/// Resolves an ordinal after the caller establishes its capture scope.
///
/// The manifest **must** belong to the reference's capture. Callers establish this through
/// [`optic_store::Store::read_instances`]. This helper checks only the ordinal, so a mismatched
/// manifest can select an unrelated instance.
fn referenced_instance<'a>(
    manifest: &'a InstanceManifest,
    reference: &InstanceRef,
) -> Result<&'a InstanceRecord, Error> {
    let instance = usize::try_from(reference.ordinal())
        .ok()
        .and_then(|ordinal| manifest.instances().get(ordinal));

    instance.ok_or_else(|| {
        error::InvalidInstanceReferenceSnafu {
            reference: reference.clone(),
            instance_count: manifest.instances().len(),
        }
        .build()
    })
}

#[cfg(test)]
mod tests;
