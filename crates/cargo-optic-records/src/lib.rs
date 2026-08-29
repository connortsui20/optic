//! Defines Cargo Optic's durable interchange format.
//!
//! Capture execution and storage use these records as their shared boundary. A [`CaptureRecord`]
//! describes one successful Cargo invocation. [`InstanceManifest`] records concrete compiler
//! instances separately, and [`CaptureId`] joins the two records without exposing store layout.
//! Both formats exclude transient process state.
//!
//! Every public record is structurally valid by construction. Constructors enforce field
//! invariants, fields are private, and deserialization uses the same validation paths. These checks
//! do not prove that recorded provenance is true. Readers accept only the format version written by
//! this release.

const CAPTURE_FORMAT_VERSION: u32 = 1;

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
