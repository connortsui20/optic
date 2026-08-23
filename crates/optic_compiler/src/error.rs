use std::path::PathBuf;

use snafu::Snafu;

/// Explains why Cargo provenance could not be produced for a request.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("failed to read Cargo metadata"))]
    Metadata { source: cargo_metadata::Error },

    #[snafu(display("package must name a workspace member, got {package}"))]
    PackageNotFound { package: String },

    #[snafu(display("package {package} must contain the selected {target} target, got no match"))]
    TargetNotFound { package: String, target: String },

    #[snafu(display("Cargo must resolve build.rustc, got {diagnostics}"))]
    CargoConfiguration { diagnostics: String },

    #[snafu(display("Cargo must return build.rustc, got no matching field"))]
    MissingCargoConfiguration,

    #[snafu(display("Cargo must report the build.rustc origin, got {line}"))]
    MissingCargoConfigurationOrigin { line: String },

    #[snafu(display("Cargo must encode build.rustc as a string, got {encoded_value}"))]
    InvalidCargoConfigurationValue {
        encoded_value: String,
        source: serde_json::Error,
    },

    #[snafu(display("rustc verbose version must contain {field}, got no value"))]
    MissingToolchainField { field: &'static str },

    #[snafu(display("failed to start {}", program.display()))]
    StartProcess {
        program: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("{} must complete successfully, got {status}", program.display()))]
    ProcessFailed { program: PathBuf, status: String },

    #[snafu(transparent)]
    Record { source: optic_records::Error },
}
