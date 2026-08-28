//! Keeps persistence failures attached to the affected operation and path.
//!
//! Typed corruption errors prevent invalid completed entries from becoming valid history.

use std::ffi::OsString;
use std::path::PathBuf;

use optic_records::CaptureId;
use snafu::Snafu;

/// Explains why the store could not publish or read a capture.
#[derive(Debug, Snafu)]
#[non_exhaustive]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// The workspace root was not absolute.
    #[snafu(display("workspace root must be absolute, got {}", path.display()))]
    WorkspaceRootNotAbsolute {
        /// The relative workspace root.
        path: PathBuf,
    },

    /// The completed namespace already contained the capture ID.
    #[snafu(display("completed capture must not already exist, got {id}"))]
    CaptureExists {
        /// The duplicate capture ID.
        id: CaptureId,
    },

    /// A filesystem operation failed before publication committed or while history was read.
    #[snafu(display("failed to {operation} {}", path.display()))]
    Filesystem {
        /// The attempted filesystem operation.
        operation: &'static str,

        /// The affected path.
        path: PathBuf,

        /// The filesystem error.
        source: std::io::Error,
    },

    /// A capture record could not be encoded or decoded.
    #[snafu(display("failed to process capture record at {}", path.display()))]
    Json {
        /// The capture record path.
        path: PathBuf,

        /// The JSON error.
        source: serde_json::Error,
    },

    /// A completed-namespace entry was not a directory.
    #[snafu(display(
        "completed capture entries must be directories, got a non-directory at {}",
        path.display()
    ))]
    ExpectedCaptureDirectory {
        /// The invalid entry path.
        path: PathBuf,
    },

    /// A capture directory name was not valid UTF-8.
    #[snafu(display(
        "capture directory name must be valid UTF-8, got {name:?} at {}",
        path.display()
    ))]
    InvalidCaptureDirectoryName {
        /// The invalid entry path.
        path: PathBuf,

        /// The non-UTF-8 directory name.
        name: OsString,
    },

    /// A capture directory name was not a canonical capture ID.
    #[snafu(display(
        "capture directory name must be a canonical capture ID, got {name} at {}",
        path.display()
    ))]
    InvalidCaptureDirectoryId {
        /// The invalid capture directory path.
        path: PathBuf,

        /// The invalid directory name.
        name: String,

        /// The capture ID parsing error.
        source: optic_records::Error,
    },

    /// A record's capture ID did not match its directory name.
    #[snafu(display(
        "record ID must match directory ID {directory_id}, got {record_id} at {}",
        path.display()
    ))]
    MismatchedCaptureId {
        /// The mismatched capture record path.
        path: PathBuf,

        /// The capture ID from the directory name.
        directory_id: CaptureId,

        /// The capture ID from the durable record.
        record_id: CaptureId,
    },
}
