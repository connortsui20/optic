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
    /// The Optic ignore file contained user configuration that initialization cannot replace.
    #[snafu(display("Optic initialization requires `*` and a newline in {}, got different contents. Preserve or move this file before capture", path.display()))]
    ConflictingIgnoreFile {
        /// The existing ignore file that needs user attention.
        path: PathBuf,
    },

    /// A store path was a symlink or had the wrong filesystem type.
    #[snafu(display("store path must be a regular {expected}, got another file type at {}", path.display()))]
    UnexpectedFileType {
        /// The expected filesystem type.
        expected: &'static str,
        /// The rejected store path.
        path: PathBuf,
    },

    /// Encoded durable data exceeded the shared reader and writer budget.
    #[snafu(display("record at {} must contain at most {limit} encoded bytes, got {actual}", path.display()))]
    RecordTooLarge {
        /// The rejected record path.
        path: PathBuf,
        /// The maximum encoded length in bytes.
        limit: u64,
        /// The observed length or the first length known to exceed the limit.
        actual: u64,
    },

    /// A candidate pointer used an unsupported revision or disagreed with its capture.
    #[snafu(display("candidate at {} must match {expected}, got {actual}", path.display()))]
    InvalidCandidate {
        /// The rejected candidate pointer path.
        path: PathBuf,
        /// The required identity or format.
        expected: String,
        /// The rejected identity or format.
        actual: String,
    },

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

    /// The requested capture did not exist in completed history.
    #[snafu(display("completed capture does not exist, got {id}"))]
    CaptureNotFound {
        /// The missing capture ID.
        id: CaptureId,
    },

    /// The records passed for publication described different captures.
    #[snafu(display("instance manifest ID must match capture ID {capture_id}, got {manifest_id}"))]
    MismatchedPublishedCaptureId {
        /// The capture record identity.
        capture_id: CaptureId,
        /// The instance manifest identity.
        manifest_id: CaptureId,
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

    /// A durable record could not be encoded or decoded.
    #[snafu(display("failed to process record at {}", path.display()))]
    Json {
        /// The durable record path.
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

    /// A completed capture's instance manifest path was not a file.
    #[snafu(display(
        "completed instance manifest must be a file, got a non-file at {}",
        path.display()
    ))]
    ExpectedInstanceFile {
        /// The invalid manifest path.
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
