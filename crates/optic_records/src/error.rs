use snafu::Snafu;

/// Explains why input could not become a trusted capture record.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display(
        "capture ID must contain exactly 32 lowercase reverse-hexadecimal characters (`k` through `z`), got {value}"
    ))]
    InvalidCaptureId { value: String },

    #[snafu(display(
        "stored capture ID must use 32 lowercase reverse-hexadecimal characters or the legacy `cap_` spelling, got {value}"
    ))]
    InvalidStoredCaptureId { value: String },

    #[snafu(display("capture format version must be {expected}, got {actual}"))]
    UnsupportedFormat { expected: u32, actual: u32 },

    #[snafu(display("{field} must contain a valid value, got {actual}"))]
    InvalidField { field: &'static str, actual: String },
}
