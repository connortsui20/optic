//! Resolves and executes the Cargo build represented by a capture request.
//!
//! The capture pipeline enters this crate through [`discover_workspace`] and [`run_build`].
//! Discovery asks Cargo for authoritative workspace metadata and retains the Cargo executable that
//! answered. Execution resolves a validated [`BuildRequest`] against that metadata, identifies the
//! rustc selected by Cargo configuration, and invokes `cargo rustc` for exactly one target.
//!
//! This crate records provenance but does not alter compilation. In particular, it does not install
//! a rustc wrapper, rewrite `RUSTFLAGS`, or infer a default package, target, or profile. A successful
//! [`CompletedBuild`] therefore describes the user's normal Cargo configuration; publication and
//! durable storage belong to the capture and store crates.

mod build;
pub use build::CompletedBuild;
pub use build::run_build;

mod error;
pub(crate) use error::CargoConfigurationSnafu;
pub use error::Error;
pub(crate) use error::InvalidCargoConfigurationValueSnafu;
pub(crate) use error::MetadataSnafu;
pub(crate) use error::MissingCargoConfigurationOriginSnafu;
pub(crate) use error::MissingCargoConfigurationSnafu;
pub(crate) use error::MissingToolchainFieldSnafu;
pub(crate) use error::PackageNotFoundSnafu;
pub(crate) use error::ProcessFailedSnafu;
pub(crate) use error::StartProcessSnafu;
pub(crate) use error::TargetNotFoundSnafu;

mod request;
pub use request::BuildRequest;
pub use request::CargoTarget;
pub use request::InvalidBuildRequest;

mod toolchain;

mod workspace;
pub use workspace::Workspace;
pub use workspace::discover_workspace;
