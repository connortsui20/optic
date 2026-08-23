//! Defines Cargo Optic's durable interchange format.
//!
//! Capture execution and storage use these records as their shared boundary. A [`CaptureRecord`]
//! describes the Cargo invocation and compiler identity for one successful build; it deliberately
//! excludes transient process state and the store's physical layout. [`CaptureId`] supplies the
//! stable identity used to join a record to other evidence in later format versions.
//!
//! Every public record is valid by construction. Constructors enforce field invariants, fields are
//! private, and deserialization passes through the same constructors before returning a value.
//! Callers can therefore trust values from this crate without remembering a separate validation
//! step.

mod build_record;
pub use build_record::BuildRecord;

mod capture_id;
pub use capture_id::CaptureId;

mod capture_record;
pub use capture_record::CaptureRecord;

mod error;
pub use error::Error;
pub(crate) use error::InvalidCaptureIdSnafu;
pub(crate) use error::InvalidFieldSnafu;
pub(crate) use error::UnsupportedFormatSnafu;

mod rustc_invocation;
pub use rustc_invocation::RustcInvocation;

mod target;
pub use target::CargoTargetKind;
pub use target::TargetRecord;

mod toolchain_record;
pub use toolchain_record::ToolchainRecord;

mod validation;

const CAPTURE_FORMAT_VERSION: u32 = 1;

#[cfg(test)]
mod tests;
