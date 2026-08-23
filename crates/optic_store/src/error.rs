use std::ffi::OsString;
use std::path::PathBuf;

use optic_records::CaptureId;
use snafu::Snafu;

/// Explains why the store could not publish or read a capture.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("completed capture must not already exist, got {id}"))]
    CaptureExists { id: CaptureId },

    #[snafu(display("failed to {operation} {}", path.display()))]
    Filesystem {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to process capture record at {}", path.display()))]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[snafu(display(
        "completed capture entries must be directories, got a non-directory at {}",
        path.display()
    ))]
    ExpectedCaptureDirectory { path: PathBuf },

    #[snafu(display(
        "capture directory name must be valid UTF-8, got {name:?} at {}",
        path.display()
    ))]
    InvalidCaptureDirectoryName { path: PathBuf, name: OsString },

    #[snafu(display("invalid capture directory ID at {}", path.display()))]
    InvalidCaptureDirectoryId {
        path: PathBuf,
        source: optic_records::Error,
    },

    #[snafu(display(
        "record ID must match directory ID {directory_id}, got {record_id} at {}",
        path.display()
    ))]
    MismatchedCaptureId {
        path: PathBuf,
        directory_id: CaptureId,
        record_id: CaptureId,
    },
}
