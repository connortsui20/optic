//! Captures compiler evidence for `cargo-optic`.
//!
//! This crate owns the Cargo, compiler, and LLVM boundary. [`compile`] runs one selected Cargo target.
//! [`ingest`] reads its retained artifacts and returns an [`EvidenceBundle`]. It does not assign
//! Optic capture IDs or persist evidence.

mod capture;
pub use capture::{
    AliasTarget, ArtifactProvenance, BodyRange, CaptureInvocation, CaptureMethod, CargoArtifact,
    CommandInvocation, CompileOutcome, CompiledCapture, EnvironmentVariable, EvidenceBundle,
    LlvmAlias, LlvmDeclaration, LlvmStage, LtoScope, ModuleEvidence, UnstableAccess,
    UnstableAccessMechanism, UnstableAccessScope, compile, ingest, require_compiled_evidence,
};

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
