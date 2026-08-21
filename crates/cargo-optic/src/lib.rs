//! Persistent compiler-evidence workflows for `cargo-optic`.
//!
//! [`Application`] connects the internal `cargo-ir` collector to a project-local evidence store.
//! The command-line binary is a thin renderer over these typed workflows. A future terminal client
//! can use the same application without parsing command output.

mod app;
pub use app::Application;

mod call_site;
pub use call_site::{CallSiteDelta, CallSiteSummary};

mod cli;
pub use cli::run_cli;

mod error;
pub use error::{Error, Result};

mod events;
pub use events::{
    CaptureEvent, CapturePhase, InspectEvent, InspectSummary, ShowEvent, ShowSummary, StreamCount,
    TEXT_CHUNK_BYTES,
};

mod ids;
pub use ids::{CaptureId, InstanceId};

mod model;
pub use cargo_ir::{
    CargoProcessEvent, LlvmStage, RemarkArgument, RemarkKind, RemarkSourceLocation, UnstableAccess,
    UnstableAccessMechanism, UnstableAccessScope,
};
pub use model::{
    ArtifactSummary, BodySetDelta, BodySetSummary, BodyView, BuildSpec, BuildTarget, CachePolicy,
    CaptureDetails, CaptureDisposition, CaptureMetadata, CaptureProfile, CaptureSummary,
    CleanSummary, CommandView, CompareView, CompilerOutput, CompilerProvenance, EnvironmentView,
    FindMatchKind, FindOptions, FindResult, GcSummary, InstanceSummary, LlvmBodySummary,
    OutputAvailability, RemarkCaptureSummary, RemarkEvidenceState, RemarkFileSummary,
    RemarkKindFilter, RemarkOptions, RemarkShowView, RemarkView, RemoveSummary, ShowView,
    SourceLocation, SourceView, StoreStatus, VerifySummary,
};

mod pending;

mod source;

mod store;

mod terminal;
