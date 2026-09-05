//! Keeps compiler failures distinct from invalid requests and record failures.
//!
//! Callers need this boundary to identify the failed phase without parsing an error message.

use std::path::PathBuf;

use snafu::Snafu;

/// Explains why selected-target compiler evidence could not be produced for a request.
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

    /// The compiler environment is not supported for collection.
    #[snafu(display("invalid compiler environment: {message}"))]
    CompilerEnvironment {
        /// The invalid configuration or missing compiler fact.
        message: String,
    },

    /// A local compiler file or manifest operation failed.
    #[snafu(display("failed to {operation} {}", path.display()))]
    Filesystem {
        /// The operation that failed.
        operation: &'static str,
        /// The affected path.
        path: PathBuf,
        /// The filesystem error.
        source: std::io::Error,
    },

    /// The selected compiler wrote an invalid instance manifest.
    #[snafu(display("invalid compiler manifest at {}: {message}", path.display()))]
    InvalidManifest {
        /// The rejected manifest path.
        path: PathBuf,
        /// The violated protocol requirement.
        message: String,
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

    /// Cargo or rustc could not start.
    #[snafu(display("failed to start {}", program.display()))]
    StartProcess {
        /// The process executable.
        program: PathBuf,
        /// The process-start error.
        source: std::io::Error,
    },

    /// Cargo or rustc exited unsuccessfully.
    #[snafu(display(
        "{} must complete successfully, got {status}{}",
        program.display(),
        diagnostics
            .as_deref()
            .filter(|diagnostics| !diagnostics.is_empty())
            .map(|diagnostics| format!("\n{diagnostics}"))
            .unwrap_or_default()
    ))]
    ProcessFailed {
        /// The process executable.
        program: PathBuf,
        /// The unsuccessful process status.
        status: String,
        /// Process diagnostics, when the caller captured them.
        diagnostics: Option<String>,
    },

    /// The successful Cargo invocation could not become a valid durable record.
    #[snafu(transparent)]
    Record {
        /// The record validation error.
        source: optic_records::Error,
    },
}
