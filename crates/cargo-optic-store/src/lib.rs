//! Owns the local persistence boundary for completed captures.
//!
//! Each Cargo workspace has one [`Store`] beneath `.optic/store`. [`Store::publish`] writes a
//! complete capture, instance manifest, and declared artifacts under `staging`. It renames that
//! directory into `captures` to make the evidence visible in one atomic namespace change. An error
//! before the rename can leave staging data, but it cannot leave a partially visible capture.
//!
//! This boundary does not guarantee persistence after a system crash or power loss. Moving the
//! workspace, store, or an ancestor directory also invalidates an open handle. The caller must open
//! a new handle after such a move.
//!
//! [`Store::list_captures`], [`Store::read_capture`], and [`Store::read_instances`] ignore staging
//! and treat every completed directory and file as untrusted. Record deserialization enforces
//! structural invariants before a value reaches the caller. Manifest and candidate reads also
//! validate every declared artifact's file type and byte length, without reparsing LLVM.
//!
//! The caller must serialize captures and prevent external store mutation during each operation.
//! [`Store::initialize`] excludes store output from ordinary Cargo package tracking before builds.
//! Interrupted staging directories and historical captures remain until the user removes them.

use std::path::Path;
use std::path::PathBuf;

const CAPTURE_FILE_NAME: &str = "capture.json";
const INSTANCES_FILE_NAME: &str = "instances.json";

/// Maximum encoded capture header or candidate pointer length: 1 MiB, including trailing whitespace.
///
/// This MVP resource budget applies to both readers and writers. It is not a compiler limit or a
/// bound on total process memory. Changes require a fixture or workload that demonstrates the need.
pub const MAX_HEADER_BYTES: u64 = 1024 * 1024;

/// Maximum encoded instance manifest length: 128 MiB, including trailing whitespace.
///
/// This MVP resource budget applies to both readers and writers under the same policy as
/// [`MAX_HEADER_BYTES`].
pub const MAX_INSTANCES_BYTES: u64 = 128 * 1024 * 1024;

mod captures;

mod artifacts;

mod candidate;

mod initialize;

mod record_io;

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
    #[cfg(test)]
    publication_failure: Option<publish::PublicationBoundary>,
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
            #[cfg(test)]
            publication_failure: None,
        })
    }
}

#[cfg(test)]
mod tests;
