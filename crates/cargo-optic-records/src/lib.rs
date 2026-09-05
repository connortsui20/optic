//! Defines Cargo Optic's durable interchange format.
//!
//! Capture execution and storage use these records as their shared boundary. A [`CaptureRecord`]
//! describes one successful selected-target compiler invocation. It excludes transient process
//! state and the store's physical layout. [`CaptureId`] supplies the stable identity used to join a
//! record to its [`InstanceManifest`].
//!
//! Every public record is structurally valid by construction. Constructors enforce field
//! invariants, fields are private, and deserialization uses the same validation paths. These checks
//! do not prove that recorded provenance is true. Readers accept only the format version written by
//! this release.

// Revision 4 requires source availability, artifact references, and complete LLVM collection state.
const CAPTURE_FORMAT_VERSION: u32 = 4;

mod artifact;
pub use artifact::ArtifactId;
pub use artifact::ArtifactKind;
pub use artifact::ArtifactRecord;

mod byte_range;
pub use byte_range::ByteRange;

mod instance_ref;
pub use instance_ref::InstanceRef;

mod source;
pub use source::SourceAvailability;
pub use source::SourceRecord;
pub use source::SourceUnavailable;

mod llvm;
pub use llvm::LlvmCollection;
pub use llvm::LlvmDefinitionKind;
pub use llvm::LlvmDefinitionRecord;
pub use llvm::LlvmModuleRecord;
pub use llvm::LlvmStage;
pub use llvm::UnsupportedLlvmConfiguration;

mod llvm_provenance;
pub use llvm_provenance::LlvmLto;
pub use llvm_provenance::LlvmProvenance;

mod analysis_token;
pub use analysis_token::AnalysisToken;

mod capture_key;
pub use capture_key::CaptureKey;

mod cargo_artifact_record;
pub use cargo_artifact_record::CargoArtifactRecord;

mod capture_analysis;
pub use capture_analysis::CaptureAnalysis;

mod build_record;
pub use build_record::BuildRecord;

mod capture_id;
pub use capture_id::CaptureId;

mod reverse_hex;

mod capture_record;
pub use capture_record::CaptureRecord;

mod compiler_identity;
pub use compiler_identity::CompilerIdentity;

mod definition_record;
pub use definition_record::DefinitionRecord;

mod error;
pub use error::Error;

mod placement_record;
pub use placement_record::PlacementRecord;

mod instance_record;
pub use instance_record::InstanceRecord;

mod instance_manifest;
pub use instance_manifest::InstanceManifest;

mod target;
pub use target::CargoTargetKind;
pub use target::TargetRecord;

mod validation;

#[cfg(test)]
mod tests;
