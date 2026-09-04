//! Provides the application boundary for Cargo Optic.
//!
//! Consumers begin with [`Optic::open`], which discovers the enclosing Cargo workspace and binds a
//! store handle to its root. The resulting [`Optic`] value exposes three product operations:
//! [`Optic::capture`] executes and publishes a [`BuildRequest`], [`Optic::find`] searches one
//! completed capture, and [`Optic::list_captures`] reads the validated completed history.
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

pub use optic_evidence::Error as EvidenceError;
pub use optic_evidence::FindResults;
pub use optic_evidence::MatchKind;
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

    /// Finds concrete compiler instances within one explicit capture.
    ///
    /// Exact definition paths, concrete display names, and raw symbols take precedence over a
    /// case-sensitive literal substring match.
    ///
    /// # Errors
    ///
    /// Returns an error if the query or limit is invalid, the capture does not exist, or its
    /// durable evidence cannot be read.
    pub fn find(
        &self,
        capture: &CaptureId,
        query: &str,
        limit: usize,
    ) -> Result<FindResults, Error> {
        Ok(optic_evidence::find_instances(
            &self.store,
            capture,
            query,
            limit,
        )?)
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
    /// Evidence search failed.
    #[snafu(transparent)]
    Evidence {
        /// The evidence subsystem error.
        source: optic_evidence::Error,
    },
    /// Store setup or completed-history reading failed.
    #[snafu(transparent)]
    Store {
        /// The store subsystem error.
        source: optic_store::Error,
    },
}
