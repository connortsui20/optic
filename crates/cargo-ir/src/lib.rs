//! Captures compiler evidence for `cargo-optic`.
//!
//! This crate owns the Cargo, compiler, and LLVM boundary. [`compile_with_events`] runs one selected
//! Cargo target and reports its user-visible output. [`compile`] is the silent convenience entry
//! point. [`ingest_with_events`] streams retained artifacts to the product store. This crate does
//! not assign Optic capture IDs or persist evidence.

mod capture;
pub use capture::{
    AliasTarget, ArtifactProvenance, BodyRange, CaptureInvocation, CaptureMethod,
    CommandInvocation, CompileOutcome, CompiledCapture, EnvironmentVariable, EvidenceEvent,
    EvidenceMetadata, LlvmAlias, LlvmDeclaration, LlvmStage, LtoScope, ModuleStart,
    RemarkFileStart, UnstableAccess, UnstableAccessMechanism, UnstableAccessScope, check_fresh,
    compile, compile_with_events, ingest_with_events, require_compiled_evidence,
};

mod cargo_output;
pub use cargo_output::CargoProcessEvent;

mod error;
pub use error::{Error, Result};

mod driver;

mod llvm;

mod mono;
pub use mono::{
    CodegenUnitPlacement, CompilerInstance, CompilerManifestReader, CompilerPlacement,
    DefinitionOrigin, SourceSpan,
};

mod request;
pub use request::{BuildRequest, CaptureProfile, CargoTarget};

mod remarks;
pub use remarks::{
    OptimizationRemark, RemarkArgument, RemarkKind, RemarkParseLimits, RemarkSourceLocation,
    parse_optimization_remarks, parse_optimization_remarks_with,
};

mod toolchain;
pub use toolchain::{Toolchain, inspect_toolchain, inspect_workspace_toolchain};
