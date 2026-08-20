//! Captures compiler evidence for `cargo-optic`.
//!
//! This crate owns the Cargo, compiler, and LLVM boundary. [`capture`] runs one selected Cargo target
//! and returns an [`EvidenceBundle`] whose paths belong to the caller-provided analysis directory.
//! It does not assign Optic capture IDs or persist evidence.

mod capture;
pub use capture::{
    AliasTarget, ArtifactProvenance, BodyRange, CaptureInvocation, CaptureMethod, CaptureOutcome,
    CargoArtifact, CommandInvocation, EnvironmentVariable, EvidenceBundle, LlvmAlias,
    LlvmDeclaration, LlvmStage, LtoScope, ModuleEvidence, UnstableAccess, UnstableAccessMechanism,
    UnstableAccessScope, capture,
};

mod error;
pub use error::{Error, Result};

mod driver;

mod llvm;

mod mono;
pub use mono::{CodegenUnitPlacement, CompilerInstance, DefinitionOrigin, SourceSpan};

mod request;
pub use request::{BuildRequest, CaptureProfile, CargoTarget};

mod toolchain;
pub use toolchain::{Toolchain, inspect_toolchain, inspect_workspace_toolchain};
