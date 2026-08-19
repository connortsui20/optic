//! Captures compiler evidence for `cargo-optic`.
//!
//! This crate owns the nightly Cargo and LLVM boundary. [`capture`] runs one selected Cargo target
//! and returns an [`EvidenceBundle`] whose paths belong to the caller-provided analysis directory.
//! It does not assign Optic capture IDs or persist evidence.

mod capture;
pub use capture::{BodyRange, EvidenceBundle, LlvmStage, ModuleEvidence, capture};

mod error;
pub use error::{Error, Result};

mod driver;

mod llvm;

mod mono;
pub use mono::CompilerInstance;

mod request;
pub use request::{BuildRequest, CargoTarget};

mod toolchain;
pub use toolchain::{Toolchain, inspect_toolchain};
