//! Provides the application boundary for Cargo Optic.
//!
//! Consumers begin with [`Optic::open`], which discovers the enclosing Cargo workspace and binds a
//! store handle to its root. The resulting [`Optic`] value exposes the two product operations:
//! [`Optic::capture`] executes and publishes a [`BuildRequest`], while [`Optic::list_captures`]
//! reads the validated completed history.
//!
//! This crate is the primary application API. The subsystem crates remain available as narrow APIs
//! for callers that need their individual boundaries. Most applications should use the types
//! re-exported here instead of coordinating compiler execution or opening `.optic` paths.
//! Command-line parsing and human-readable rendering remain outside this crate.

use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub use optic_compiler::BuildRequest;
pub use optic_compiler::CargoTarget;
pub use optic_compiler::Error as CompilerError;
pub use optic_compiler::InvalidBuildRequest;
use optic_compiler::Workspace;

pub use optic_records::BuildRecord;
pub use optic_records::CaptureId;
pub use optic_records::CaptureRecord;
pub use optic_records::CargoTargetKind;
pub use optic_records::Error as RecordError;
pub use optic_records::TargetRecord;
pub use optic_store::Error as StoreError;
use optic_store::Store;

use snafu::ResultExt;
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

    /// Runs and publishes one new immutable Cargo invocation capture.
    ///
    /// # Errors
    ///
    /// Returns an error if Cargo cannot complete the request, the system clock cannot produce the
    /// record timestamp, or the record cannot be published.
    pub fn capture(&self, request: &BuildRequest) -> Result<CaptureRecord, Error> {
        let build = optic_compiler::run_build(&self.workspace, request)?;

        let completed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context(ClockSnafu)?
            .as_millis()
            .try_into()
            .context(TimestampOverflowSnafu)?;

        let record = CaptureRecord::new(CaptureId::generate(), completed_at_unix_ms, build);

        self.store.publish(&record)?;

        Ok(record)
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
    /// Cargo workspace discovery or build execution failed.
    #[snafu(transparent)]
    Compiler {
        /// The compiler subsystem error.
        source: optic_compiler::Error,
    },

    /// The system clock was earlier than the Unix epoch.
    #[snafu(display("system clock must be at or after the Unix epoch"))]
    Clock {
        /// The invalid system time.
        source: std::time::SystemTimeError,
    },

    /// The completion timestamp did not fit in the record field.
    #[snafu(display("capture timestamp must fit in u64 milliseconds, got an overflow"))]
    TimestampOverflow {
        /// The integer conversion error.
        source: std::num::TryFromIntError,
    },

    /// Store setup, capture publication, or history reading failed.
    #[snafu(transparent)]
    Store {
        /// The store subsystem error.
        source: optic_store::Error,
    },
}
