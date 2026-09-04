//! Provides the application boundary for Cargo Optic.
//!
//! Consumers begin with [`Optic::open`], which discovers the enclosing Cargo workspace and binds a
//! store handle to its root. The resulting [`Optic`] value exposes two product operations:
//! [`Optic::capture`] executes and publishes a [`BuildRequest`], while [`Optic::list_captures`]
//! reads the validated completed history.
//!
//! This crate is the primary application API. The subsystem crates remain available as narrow APIs
//! for callers that need their individual boundaries. Most applications should use the types
//! re-exported here instead of coordinating compiler execution or opening `.optic` paths.
//! Command-line parsing and human-readable rendering remain outside this crate.

use std::path::Path;

pub use optic_capture::Error as CaptureError;
pub use optic_compiler::BuildRequest;
pub use optic_compiler::CargoTarget;
pub use optic_compiler::Error as CompilerError;
pub use optic_compiler::InvalidBuildRequest;
use optic_compiler::Workspace;

pub use optic_records::BuildRecord;
pub use optic_records::CaptureId;
pub use optic_records::CaptureRecord;
pub use optic_records::CargoTargetKind;
pub use optic_records::CompilerIdentity;
pub use optic_records::DefinitionRecord;
pub use optic_records::Error as RecordError;
pub use optic_records::InstanceRecord;
pub use optic_records::PlacementRecord;
pub use optic_records::TargetRecord;
pub use optic_store::Error as StoreError;
use optic_store::Store;

use snafu::Snafu;

/// Cargo Optic operations for one discovered workspace.
///
/// The invocation directory, workspace root, and member paths must keep their locations for the
/// lifetime of this value. Open a new value after moving the workspace.
pub struct Optic {
    workspace: Workspace,
    store: Store,
}

impl Optic {
    /// Opens the Cargo workspace containing the absolute `start` path.
    ///
    /// This call discovers Cargo metadata but does not create the Optic store.
    ///
    /// # Errors
    ///
    /// Returns an error if `start` is relative or Cargo cannot discover the workspace.
    pub fn open(start: &Path) -> Result<Self, Error> {
        let workspace = optic_compiler::discover_workspace(start)?;
        let store = Store::new(workspace.root())?;

        Ok(Self { workspace, store })
    }

    /// Runs and publishes one new immutable selected-target capture.
    ///
    /// # Errors
    ///
    /// Returns an error if compiler collection fails, the system clock cannot produce the record
    /// timestamp, record validation fails, or the complete capture cannot be published.
    pub fn capture(&self, request: &BuildRequest) -> Result<CaptureRecord, Error> {
        Ok(optic_capture::capture(
            &self.workspace,
            &self.store,
            request,
        )?)
    }

    /// Lists captures by descending recorded completion time, then ascending capture ID.
    ///
    /// # Errors
    ///
    /// Returns an error if any completed durable record is invalid or cannot be read.
    pub fn list_captures(&self) -> Result<Vec<CaptureRecord>, Error> {
        Ok(self.store.list_captures()?)
    }
}

/// Explains why an application-level Cargo Optic operation failed.
#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum Error {
    /// Cargo workspace discovery failed while opening the application.
    #[snafu(transparent)]
    Compiler {
        /// The compiler subsystem error.
        source: optic_compiler::Error,
    },
    /// Capture planning, compiler collection, validation, or publication failed.
    #[snafu(transparent)]
    Capture {
        /// The capture subsystem error.
        source: optic_capture::Error,
    },
    /// Store setup or completed-history reading failed.
    #[snafu(transparent)]
    Store {
        /// The store subsystem error.
        source: optic_store::Error,
    },
}
