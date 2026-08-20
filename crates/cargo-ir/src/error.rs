//! Errors returned by compiler-evidence operations.
//!
//! These errors preserve the failed program, compiler diagnostics, and affected filesystem path.

use std::io;
use std::path::PathBuf;

/// A compiler-evidence failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The compiler environment cannot support an exact capture.
    #[error("invalid compiler environment: {message}")]
    CompilerEnvironment {
        /// The failed environment requirement.
        message: String,
    },

    /// Cargo or rustc could not be started.
    #[error("failed to start {program}: {source}")]
    StartProcess {
        /// The program that could not be started.
        program: String,

        /// The operating-system error.
        source: io::Error,
    },

    /// A compiler subprocess returned an error.
    #[error("{program} failed with status {status}\n{diagnostics}")]
    ProcessFailed {
        /// The failed program.
        program: String,

        /// The printable exit status.
        status: String,

        /// Human-readable subprocess diagnostics.
        diagnostics: String,
    },

    /// A required value was absent from compiler version output.
    #[error("rustc -vV did not report {field}")]
    MissingToolchainField {
        /// The missing field name.
        field: &'static str,
    },

    /// The analyzed toolchain does not contain `llvm-dis`.
    #[error(
        "the selected compiler does not contain llvm-dis at {path}{}",
        install_command.as_ref().map(|command| format!("; run `{command}`")).unwrap_or_else(|| "; install its matching llvm-tools component".to_owned())
    )]
    MissingLlvmDis {
        /// The expected executable path.
        path: PathBuf,

        /// The exact rustup command, when the sysroot identifies its toolchain.
        install_command: Option<String>,
    },

    /// The active toolchain does not contain compiler libraries for an external driver.
    #[error(
        "the selected compiler does not contain rustc-dev libraries at {path}{}",
        install_command.as_ref().map(|command| format!("; run `{command}`")).unwrap_or_else(|| "; install its matching rustc-dev component".to_owned())
    )]
    MissingRustcDev {
        /// The directory that must contain rustc compiler libraries.
        path: PathBuf,

        /// The exact rustup command, when the sysroot identifies its toolchain.
        install_command: Option<String>,
    },

    /// A filesystem operation failed.
    #[error("failed to {operation} {path}: {source}")]
    Filesystem {
        /// The attempted operation.
        operation: &'static str,

        /// The affected path.
        path: PathBuf,

        /// The operating-system error.
        source: io::Error,
    },

    /// The analysis directory contains files from an earlier operation.
    #[error("analysis directory must be empty, got {path}")]
    AnalysisDirectoryNotEmpty {
        /// The directory that contains existing files.
        path: PathBuf,
    },

    /// An LLVM module was not structurally complete.
    #[error("invalid LLVM IR in {path}: {message}")]
    InvalidLlvm {
        /// The invalid textual module.
        path: PathBuf,

        /// The structural error.
        message: String,
    },

    /// A compiler identity manifest did not satisfy its protocol.
    #[error("invalid compiler identity manifest in {path}: {message}")]
    InvalidIdentityManifest {
        /// The invalid manifest path.
        path: PathBuf,

        /// The failed protocol requirement.
        message: String,
    },

    /// Cargo completed a compilation without the required artifacts.
    #[error("Cargo compiled the selected target but produced no supported LLVM artifacts")]
    MissingEvidence,
}

/// A compiler-evidence result.
pub type Result<T> = std::result::Result<T, Error>;
