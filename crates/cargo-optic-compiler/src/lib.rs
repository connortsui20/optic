//! Resolves and executes the Cargo build represented by a capture request.
//!
//! The capture pipeline enters this crate through [`discover_workspace`] and [`run_build`].
//! Discovery asks Cargo for authoritative workspace metadata and retains the Cargo executable that
//! answered. Execution resolves a validated [`BuildRequest`] against metadata with the requested
//! feature selection and invokes `cargo rustc` for exactly one target with the same selection.
//!
//! This crate records provenance but does not alter compilation. In particular, it does not
//! install a rustc wrapper, rewrite `RUSTFLAGS`, or infer a default package, target, or profile. A
//! successful [`optic_records::BuildRecord`] therefore describes the successful Cargo invocation.
//! It does not identify the compiler or claim that Cargo invoked rustc. Publication and durable
//! storage belong to the application and store crates.

mod build;
pub use build::run_build;

mod error;
pub use error::Error;

mod request;
pub use request::BuildRequest;
pub use request::CargoTarget;
pub use request::InvalidBuildRequest;

mod workspace;
pub use workspace::Workspace;
pub use workspace::discover_workspace;
