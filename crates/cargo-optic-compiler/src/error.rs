//! Keeps compiler failures distinct from invalid requests and record failures.
//!
//! Callers need this boundary to identify the failed phase without parsing an error message.

use std::path::PathBuf;

use snafu::Snafu;

/// Explains why a Cargo invocation record could not be produced for a request.
#[derive(Debug, Snafu)]
#[non_exhaustive]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// The invocation directory was not absolute.
    #[snafu(display("invocation directory must be absolute, got {}", path.display()))]
    InvocationDirectoryNotAbsolute {
        /// The relative invocation directory.
        path: PathBuf,
    },

    /// Cargo metadata could not describe the requested workspace.
    #[snafu(display("failed to read Cargo metadata"))]
    Metadata {
        /// The Cargo metadata error.
        source: cargo_metadata::Error,
    },

    /// The requested package was not a workspace member.
    #[snafu(display("package must name a workspace member, got {package}"))]
    PackageNotFound {
        /// The requested package name.
        package: String,
    },

    /// The requested package did not contain the selected target.
    #[snafu(display("package {package} must contain the selected {target} target, got no match"))]
    TargetNotFound {
        /// The resolved package name.
        package: String,

        /// The requested target description.
        target: String,
    },

    /// Cargo could not start.
    #[snafu(display("failed to start {}", program.display()))]
    StartProcess {
        /// The selected Cargo executable.
        program: PathBuf,

        /// The process-start error.
        source: std::io::Error,
    },

    /// Cargo exited without completing the selected target successfully.
    #[snafu(display("{} must complete successfully, got {status}", program.display()))]
    ProcessFailed {
        /// The selected Cargo executable.
        program: PathBuf,

        /// The unsuccessful process status.
        status: String,
    },

    /// The successful Cargo invocation could not become a valid durable record.
    #[snafu(transparent)]
    Record {
        /// The record validation error.
        source: optic_records::Error,
    },
}
