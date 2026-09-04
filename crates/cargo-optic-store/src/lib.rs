//! Owns the local persistence boundary for completed captures.
//!
//! Each Cargo workspace has one [`Store`] beneath `.optic/store`. [`Store::publish`] writes a
//! complete capture and instance manifest under `staging`. It renames that directory into
//! `captures` to make both records visible in one atomic namespace change. A process error before
//! the rename can leave staging data, but it cannot leave a partially visible capture.
//!
//! This boundary does not guarantee persistence after a system crash or power loss. Moving the
//! workspace, store, or an ancestor directory also invalidates an open handle. The caller must open
//! a new handle after such a move.
//!
//! [`Store::list_captures`], [`Store::read_capture`], and [`Store::read_instances`] ignore staging
//! and treat every completed directory and file as untrusted. Record deserialization enforces
//! structural invariants before a value reaches the caller.

use std::path::Path;
use std::path::PathBuf;

const CAPTURE_FILE_NAME: &str = "capture.json";
const INSTANCES_FILE_NAME: &str = "instances.json";

mod captures;

mod error;
pub use error::Error;

mod publish;

/// The `.optic/store` capture store for one Cargo workspace.
///
/// The workspace root and its ancestors must keep their locations for the lifetime of this value.
/// The caller must open a new value after moving any of them.
pub struct Store {
    /// The derived `.optic/store` root used by every operation on this handle.
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
