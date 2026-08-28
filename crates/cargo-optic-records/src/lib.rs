//! Defines Cargo Optic's durable interchange format.
//!
//! Capture execution and storage use these records as their shared boundary. A [`CaptureRecord`]
//! describes one successful Cargo invocation. It deliberately excludes transient process state,
//! compiler identity, and the store's physical layout. [`CaptureId`] supplies the stable identity
//! used to join a record to other evidence.
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

mod capture_record;
pub use capture_record::CaptureRecord;

mod error;
pub use error::Error;

mod reverse_hex;

mod target;
pub use target::CargoTargetKind;
pub use target::TargetRecord;

mod validation;

#[cfg(test)]
mod tests;
