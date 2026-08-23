use snafu::Snafu;

/// Explains why input could not become a trusted capture record.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display(
        "capture ID must be `cap_` followed by 32 lowercase hexadecimal characters, got {value}"
    ))]
    InvalidCaptureId { value: String },

    #[snafu(display("capture format version must be {expected}, got {actual}"))]
    UnsupportedFormat { expected: u32, actual: u32 },

    #[snafu(display("{field} must contain a valid value, got {actual}"))]
    InvalidField { field: &'static str, actual: String },
}
