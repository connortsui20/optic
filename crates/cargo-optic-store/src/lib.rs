//! Owns the local persistence boundary for completed captures.
//!
//! Each Cargo workspace has one [`Store`] beneath `.optic/store`. [`Store::publish`] writes a
//! complete record under `staging`. It renames that directory into `captures` to make the record
//! visible in one atomic namespace change. A process error before the rename can leave staging
//! data, but it cannot leave a partially visible capture.
//!
//! This boundary does not guarantee persistence after a system crash or power loss. Moving the
//! workspace, store, or an ancestor directory also invalidates an open handle. The caller must open
//! a new handle after such a move.
//!
//! [`Store::captures`] ignores staging and treats every completed directory and file as untrusted.
//! Record deserialization enforces structural invariants before a value reaches the caller.

use std::path::Path;
use std::path::PathBuf;

mod captures;

mod error;
pub use error::Error;

mod publish;

const RECORD_FILE_NAME: &str = "capture.json";

/// The `.optic/store` capture store for one Cargo workspace.
///
/// The workspace root and its ancestors must keep their locations for the lifetime of this value.
/// The caller must open a new value after moving any of them.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Creates a workspace store handle without creating persistent state.
    ///
    /// # Errors
    ///
    /// Returns an error if `workspace_root` is relative.
    pub fn new(workspace_root: &Path) -> Result<Self, Error> {
        if !workspace_root.is_absolute() {
            return error::WorkspaceRootNotAbsoluteSnafu {
                path: workspace_root.to_owned(),
            }
            .fail();
        }

        Ok(Self {
            root: workspace_root.join(".optic").join("store"),
        })
    }
}

#[cfg(test)]
mod tests;
