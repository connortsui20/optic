//! Captures compiler evidence for `cargo-optic`.
//!
//! This crate owns the Cargo, compiler, and LLVM boundary. [`compile_with_events`] runs one selected
//! Cargo target and reports its user-visible output. [`compile`] is the silent convenience entry
//! point. [`ingest`] reads retained artifacts and returns an [`EvidenceBundle`]. This crate does not
//! assign Optic capture IDs or persist evidence.

mod capture;
pub use capture::{
    AliasTarget, ArtifactProvenance, BodyRange, CaptureInvocation, CaptureMethod,
    CommandInvocation, CompileOutcome, CompiledCapture, EnvironmentVariable, EvidenceBundle,
    LlvmAlias, LlvmDeclaration, LlvmStage, LtoScope, ModuleEvidence, RemarkEvidence,
    UnstableAccess, UnstableAccessMechanism, UnstableAccessScope, check_fresh, compile,
    compile_with_events, ingest, require_compiled_evidence,
};

mod cargo_output;
pub use cargo_output::CargoProcessEvent;

mod error;
pub use error::{Error, Result};

mod driver;

mod llvm;

mod mono;
pub use mono::{CodegenUnitPlacement, CompilerInstance, DefinitionOrigin, SourceSpan};

mod request;
pub use request::{BuildRequest, CaptureProfile, CargoTarget};

mod remarks;
pub use remarks::{
    OptimizationRemark, RemarkArgument, RemarkKind, RemarkParseLimits, RemarkSourceLocation,
    parse_optimization_remarks,
};

mod toolchain;
pub use toolchain::{Toolchain, inspect_toolchain, inspect_workspace_toolchain};
