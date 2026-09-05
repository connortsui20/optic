//! Resolves and executes the Cargo build represented by a capture request.
//!
//! The capture pipeline enters this crate through [`discover_workspace`] and [`prepare_build`].
//! Discovery asks Cargo for authoritative workspace metadata and retains the Cargo executable that
//! answered. Preparation resolves a validated [`BuildRequest`] against metadata and provisions a
//! cached driver with the exact default compiler. [`PreparedBuild`] then probes a candidate token
//! or collects with a new token through `cargo rustc`.
//!
//! Cargo's public metadata does not expose concrete function instances or codegen-unit placement.
//! The compiler crate therefore embeds a standalone `rustc_private` driver and compiles it with the
//! exact rustc selected by Cargo. `rustc_private` means rustc's internal implementation crates, not
//! a different compiler executable.
//!
//! Collection disables configured compiler wrappers with a warning. The driver observes only the
//! selected-target compiler invocation. A probe stops before analysis if Cargo requests compilation.
//! A successful [`CollectedBuild`] proves that this compiler ran successfully and returned a complete
//! instance manifest. Publication and durable storage belong to the capture and store crates.
//! Captured source and complete optimized LLVM modules remain in an owned temporary directory until
//! publication. Unsupported LLVM configurations retain source and concrete-instance evidence.

mod build;

mod attempt;

mod observation;

mod prepared;
pub use prepared::Freshness;
pub use prepared::PreparedBuild;
pub use prepared::prepare_build;

mod collection;
pub use collection::CollectedBuild;

mod artifacts;

mod llvm_index;

mod driver;

mod digest;

mod error;
pub use error::Error;

mod manifest;

#[path = "../rustc-driver/protocol.rs"]
mod protocol;

mod request;
pub use request::BuildRequest;
pub use request::CargoTarget;
pub use request::InvalidBuildRequest;

mod workspace;
pub use workspace::Workspace;
pub use workspace::discover_workspace;

mod toolchain;

#[cfg(test)]
mod tests;
