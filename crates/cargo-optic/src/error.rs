//! Errors returned by Optic application workflows.
//!
//! The variants separate compiler, catalog, request, and filesystem failures for library callers.

use std::io;
use std::path::PathBuf;
use std::time::SystemTimeError;

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

    /// Cargo did not invoke rustc and no matching verified evidence exists.
    #[error("compiler evidence is unavailable: {message}")]
    EvidenceUnavailable {
        /// The action that restores evidence.
        message: String,
    },

    /// No stored capture matches the requested full ID or prefix.
    #[error("capture ID must match one stored capture, got {capture_id}")]
    UnknownCapture {
        /// The unmatched capture ID or prefix.
        capture_id: CaptureId,
    },

    /// No stored instance matches the requested full ID or prefix.
    #[error("instance ID must match one stored instance, got {instance_id}")]
    UnknownInstance {
        /// The unmatched instance ID or prefix.
        instance_id: InstanceId,
    },

    /// An ID prefix matches more than one stored ID.
    #[error(
        "{kind} ID prefix must match only one stored ID. Use more characters. \
         Matches include {candidates}, got {prefix}"
    )]
    AmbiguousIdentifier {
        /// The type of ID that was matched.
        kind: &'static str,

        /// The ambiguous user-supplied prefix.
        prefix: String,

        /// Example matching full IDs.
        candidates: String,
    },

    /// A known build input changed while the compiler was reading it.
    #[error("build input changed during capture, got {path}")]
    InputChanged {
        /// The changed input path.
        path: PathBuf,
    },

    /// Retained compiler evidence does not satisfy the pending format.
    #[error("invalid pending compiler evidence in {path}: {message}")]
    InvalidPendingEvidence {
        /// The pending marker that contains invalid data.
        path: PathBuf,

        /// The failed format requirement.
        message: String,
    },

    /// The system clock is before the Unix epoch.
    #[error("system clock must be after the Unix epoch, got {source}")]
    SystemClock {
        /// The invalid clock duration.
        source: SystemTimeError,
    },

    /// The project store has an unsupported format.
    #[error(
        ".optic store format must be {expected}, got {actual}\n\
         Run `cargo optic clean` to recreate the store"
    )]
    StoreVersion {
        /// The current store format.
        expected: u32,

        /// The format found on disk.
        actual: u32,
    },

    /// Evidence from the previous store layout exists at the `.optic` root.
    #[error(
        "legacy Optic evidence exists at {path}\n\
         Run `cargo optic clean` to recreate the store"
    )]
    LegacyStore {
        /// One legacy evidence path that was found.
        path: PathBuf,
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

    /// A numeric value does not fit in its stored representation.
    #[error("{name} must be at most {maximum}, got {actual}")]
    IntegerOutOfRange {
        /// The value name.
        name: &'static str,

        /// The largest supported value.
        maximum: u128,

        /// The unsupported value.
        actual: u128,
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
