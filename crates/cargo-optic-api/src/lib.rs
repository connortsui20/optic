//! Provides the application boundary for Cargo Optic.
//!
//! Consumers begin with [`Optic::open`], which discovers the enclosing Cargo workspace and binds a
//! store handle to its root. The resulting [`Optic`] value exposes the stored-evidence workflow:
//! [`Optic::capture`] reuses or publishes a [`BuildRequest`], [`Optic::find`] searches one
//! completed capture, and [`Optic::list_captures`] reads the validated completed history.
//! [`Optic::source`] and [`Optic::llvm`] select capture-scoped evidence ranges, which
//! [`Optic::copy_evidence`] copies to a caller's writer without rereading the checkout.
//!
//! This crate is the primary application API. The subsystem crates remain available as narrow APIs
//! for callers that need their individual boundaries. Most applications should use the types
//! re-exported here instead of coordinating compiler execution or opening `.optic` paths.
//! Command-line parsing and human-readable rendering remain outside this crate.

use std::io::Write;
use std::path::Path;

pub use optic_capture::CaptureOutcome;
pub use optic_capture::CapturePolicy;
pub use optic_capture::Error as CaptureError;
pub use optic_compiler::BuildRequest;
pub use optic_compiler::CargoTarget;
pub use optic_compiler::Error as CompilerError;
pub use optic_compiler::InvalidBuildRequest;
use optic_compiler::Workspace;

pub use optic_evidence::Error as EvidenceError;
pub use optic_evidence::EvidenceRange;
pub use optic_evidence::FindResults;
pub use optic_evidence::FoundInstance;
pub use optic_evidence::LlvmBody;
pub use optic_evidence::LlvmEvidence;
pub use optic_evidence::MatchKind;
pub use optic_evidence::SourceEvidence;
pub use optic_records::ArtifactId;
pub use optic_records::BuildRecord;
pub use optic_records::ByteRange;
pub use optic_records::CaptureId;
pub use optic_records::CaptureRecord;
pub use optic_records::CargoTargetKind;
pub use optic_records::CompilerIdentity;
pub use optic_records::DefinitionRecord;
pub use optic_records::Error as RecordError;
pub use optic_records::InstanceRecord;
pub use optic_records::InstanceRef;
pub use optic_records::LlvmStage;
pub use optic_records::PlacementRecord;
pub use optic_records::SourceUnavailable;
pub use optic_records::TargetRecord;
pub use optic_records::UnsupportedLlvmConfiguration;
pub use optic_store::Error as StoreError;
use optic_store::Store;

use snafu::Snafu;

/// Cargo Optic operations for one discovered workspace.
///
/// The invocation directory, workspace root, and member paths **must** keep their locations for the
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

    /// Reuses a Cargo-fresh capture or publishes new selected-target evidence.
    ///
    /// [`CapturePolicy::Fresh`] forces selected-target analysis without discarding compatible
    /// driver or dependency artifacts. [`CaptureOutcome::Reused`] preserves the original record's
    /// ID and completion time. The caller **must** obey the writer constraints in
    /// [`optic_capture::capture`].
    ///
    /// # Errors
    ///
    /// Returns an error if request preparation, freshness verification, compiler collection,
    /// durable validation, or publication fails.
    pub fn capture(
        &self,
        request: &BuildRequest,
        policy: CapturePolicy,
    ) -> Result<CaptureOutcome, Error> {
        Ok(optic_capture::capture(
            &self.workspace,
            &self.store,
            request,
            policy,
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

    /// Selects the compiler-loaded source snapshot for one exact captured instance.
    ///
    /// Unsupported source provenance is a typed unavailable result. This operation does not
    /// compile or read current source files.
    ///
    /// # Errors
    ///
    /// Returns an error if the reference is out of bounds or its stored evidence is invalid.
    pub fn source(&self, instance: &InstanceRef) -> Result<SourceEvidence, Error> {
        Ok(optic_evidence::source_evidence(&self.store, instance)?)
    }

    /// Selects every exact standalone LLVM body for one captured compiler symbol.
    ///
    /// Results retain module, optimization stage, and direct-alias provenance. A missing exact
    /// definition does not establish that optimization eliminated the function. This operation
    /// reads the completed capture without compiling or probing freshness.
    ///
    /// # Errors
    ///
    /// Returns an error if the reference is out of bounds or its stored evidence is invalid.
    pub fn llvm(&self, instance: &InstanceRef) -> Result<LlvmEvidence, Error> {
        Ok(optic_evidence::llvm_evidence(&self.store, instance)?)
    }

    /// Copies a finite captured range through a fixed-size buffer.
    ///
    /// Obtain the range from [`Self::source`] or [`Self::llvm`]. The store checks its capture,
    /// artifact, and byte bounds before copying. A writer failure can leave partial output.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid or truncated stored evidence, failed reads, or writer errors.
    pub fn copy_evidence(
        &self,
        evidence: &EvidenceRange,
        writer: &mut impl Write,
    ) -> Result<(), Error> {
        Ok(self.store.copy_evidence(
            evidence.capture_id(),
            evidence.artifact(),
            evidence.range(),
            writer,
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
    /// Evidence search or exact-instance resolution failed.
    #[snafu(transparent)]
    Evidence {
        /// The evidence subsystem error.
        source: optic_evidence::Error,
    },
    /// Store setup, completed-history reading, or evidence copying failed.
    #[snafu(transparent)]
    Store {
        /// The store subsystem error.
        source: optic_store::Error,
    },
}
