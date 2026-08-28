//! Keeps invalid records from crossing the durable interchange boundary.
//!
//! Typed failures let constructors and deserializers enforce the same record contracts.

use snafu::Snafu;

/// Explains why input could not become a trusted capture record.
#[derive(Debug, Snafu)]
#[non_exhaustive]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// A capture ID did not use the canonical reverse-hexadecimal representation.
    #[snafu(display(
        "capture ID must contain exactly 32 lowercase reverse-hexadecimal characters \
         (`k` through `z`), got {value}"
    ))]
    InvalidCaptureId {
        /// The rejected capture ID.
        value: String,
    },

    /// A durable record used an unsupported format version.
    #[snafu(display("capture format version must be {expected}, got {actual}"))]
    UnsupportedFormat {
        /// The format version that this reader accepts.
        expected: u32,

        /// The format version in the record.
        actual: u32,
    },

    /// A record field did not satisfy its construction invariant.
    #[snafu(display("{field} must contain a valid value, got {actual}"))]
    InvalidField {
        /// The name of the invalid field.
        field: &'static str,

        /// A description of the rejected value.
        actual: String,
    },
}
