//! Errors returned by Optic application workflows.
//!
//! The variants separate compiler, catalog, request, and filesystem failures for library callers.

use std::io;
use std::path::PathBuf;

use crate::{CaptureId, InstanceId};

/// An Optic application failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Compiler evidence could not be captured or interpreted.
    #[error(transparent)]
    Compiler(#[from] cargo_ir::Error),

    /// Cargo metadata could not be read.
    #[error("failed to read Cargo metadata: {0}")]
    CargoMetadata(#[from] cargo_metadata::Error),

    /// The evidence catalog operation failed.
    #[error("evidence catalog operation failed: {0}")]
    Database(#[from] rusqlite::Error),

    /// A JSON value could not be encoded or decoded.
    #[error("failed to process JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// A filesystem operation failed.
    #[error("failed to {operation} {path}: {source}")]
    Filesystem {
        /// The attempted operation.
        operation: &'static str,

        /// The affected path.
        path: PathBuf,

        /// The operating-system error.
        source: io::Error,
    },

    /// The command combines incompatible selections.
    #[error("invalid request: {message}")]
    InvalidRequest {
        /// The rejected request detail.
        message: String,
    },

    /// The requested capture does not exist in this project store.
    #[error("capture {capture_id} does not exist")]
    UnknownCapture {
        /// The unknown capture identifier.
        capture_id: CaptureId,
    },

    /// The requested instance does not exist in this project store.
    #[error("instance {instance_id} does not exist")]
    UnknownInstance {
        /// The unknown instance identifier.
        instance_id: InstanceId,
    },

    /// A known build input changed while the compiler was reading it.
    #[error("build input changed during capture, got {path}")]
    InputChanged {
        /// The changed input path.
        path: PathBuf,
    },

    /// The project store has an unsupported format.
    #[error(
        ".optic store format must be {expected}, got {actual}\nRemove .optic to recreate the store"
    )]
    StoreVersion {
        /// The only supported format.
        expected: u32,

        /// The format found on disk.
        actual: u32,
    },

    /// A stored byte range is invalid.
    #[error("stored byte range must be valid for {path}, got {start}..{end}")]
    InvalidRange {
        /// The affected blob path.
        path: PathBuf,

        /// The inclusive start offset.
        start: u64,

        /// The exclusive end offset.
        end: u64,
    },
}

impl Error {
    pub(crate) fn filesystem(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: io::Error,
    ) -> Self {
        Self::Filesystem {
            operation,
            path: path.into(),
            source,
        }
    }
}

/// An Optic application result.
pub type Result<T> = std::result::Result<T, Error>;
