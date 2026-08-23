//! Provides the application boundary for Cargo Optic.
//!
//! Consumers begin with [`Optic::open`], which discovers the enclosing Cargo workspace and binds a
//! store handle to its root. The resulting [`Optic`] value exposes the two product operations:
//! [`Optic::capture`] executes and publishes a [`BuildRequest`], while [`Optic::captures`] reads the
//! validated completed history.
//!
//! The lower-level crates remain implementation details. Applications should use the request and
//! record types re-exported here rather than coordinating compiler execution or opening `.optic`
//! paths themselves. Command-line parsing and human-readable rendering are deliberately excluded;
//! the `cargo_optic` binary is one consumer of this API.

use std::path::Path;

pub use optic_compiler::BuildRequest;
pub use optic_compiler::CargoTarget;
pub use optic_compiler::InvalidBuildRequest;
use optic_compiler::Workspace;
pub use optic_records::CaptureId;
pub use optic_records::CaptureRecord;
pub use optic_records::CargoTargetKind;
use optic_store::Store;
use snafu::Snafu;

/// Cargo Optic operations for one discovered workspace.
pub struct Optic {
    workspace: Workspace,
    store: Store,
}

impl Optic {
    /// Opens the Cargo workspace containing `start`.
    ///
    /// This call discovers Cargo metadata but does not create the Optic store.
    ///
    /// # Errors
    ///
    /// Returns an error if Cargo cannot discover the workspace.
    pub fn open(start: &Path) -> Result<Self, Error> {
        let workspace = optic_compiler::discover_workspace(start)?;
        let store = Store::open(workspace.root());

        Ok(Self { workspace, store })
    }

    /// Builds and publishes one new immutable capture.
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot build or its record cannot be published.
    pub fn capture(&self, request: &BuildRequest) -> Result<CaptureRecord, Error> {
        Ok(optic_capture::capture(
            &self.workspace,
            &self.store,
            request,
        )?)
    }

    /// Lists completed captures from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error if any completed durable record is invalid or cannot be read.
    pub fn captures(&self) -> Result<Vec<CaptureRecord>, Error> {
        Ok(self.store.captures()?)
    }
}

/// Explains why an application-level Cargo Optic operation failed.
#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(transparent)]
    Compiler { source: optic_compiler::Error },

    #[snafu(transparent)]
    Capture { source: optic_capture::Error },

    #[snafu(transparent)]
    Store { source: optic_store::Error },
}
