//! Owns the local persistence boundary for completed captures.
//!
//! Each Cargo workspace has one [`Store`] beneath `.optic/store`. [`Store::publish`] writes a full
//! record under `staging`; renaming that directory into `captures` is the single step that makes the
//! record visible. A process that fails before the rename can leave staging data, but it cannot
//! leave a partially visible capture.
//!
//! [`Store::captures`] ignores staging and treats every completed directory and file as untrusted.
//! Record deserialization enforces the durable invariants before a value reaches the caller. The
//! layout remains private so it can evolve without changing the product API.

use std::path::Path;
use std::path::PathBuf;

mod captures;

mod error;
pub(crate) use error::CaptureExistsSnafu;
pub use error::Error;
pub(crate) use error::ExpectedCaptureDirectorySnafu;
pub(crate) use error::FilesystemSnafu;
pub(crate) use error::InvalidCaptureDirectoryIdSnafu;
pub(crate) use error::InvalidCaptureDirectoryNameSnafu;
pub(crate) use error::JsonSnafu;
pub(crate) use error::MismatchedCaptureIdSnafu;

mod publish;

const RECORD_FILE_NAME: &str = "capture.json";

/// The capture store for one Cargo workspace.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Opens a store handle without creating durable state.
    #[must_use]
    pub fn open(workspace_root: &Path) -> Self {
        Self {
            root: workspace_root.join(".optic").join("store"),
        }
    }
}

#[cfg(test)]
mod tests;
