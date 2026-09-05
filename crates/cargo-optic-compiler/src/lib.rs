//! Resolves and executes the Cargo build represented by a capture request.
//!
//! The capture pipeline enters this crate through [`discover_workspace`] and [`collect_build`].
//! Discovery asks Cargo for authoritative workspace metadata and retains the Cargo executable that
//! answered. Collection resolves a validated [`BuildRequest`] against metadata, compiles an
//! exact-version driver with the default `rustc`, and invokes `cargo rustc` with the requested
//! feature selection.
//!
//! Cargo's public metadata does not expose concrete function instances or codegen-unit placement.
//! The compiler crate therefore embeds a standalone `rustc_private` driver and compiles it with the
//! exact rustc selected by Cargo. `rustc_private` means rustc's internal implementation crates, not
//! a different compiler executable.
//!
//! Collection disables configured compiler wrappers with a warning. The driver observes only the
//! selected-target compiler invocation. A successful [`CollectedBuild`] proves that this compiler
//! ran successfully and returned a complete instance manifest. Publication and durable storage
//! belong to the capture and store crates.

mod build;
pub use build::run_build;

mod collection;
pub use collection::CollectedBuild;
pub use collection::collect_build;

mod driver;

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
