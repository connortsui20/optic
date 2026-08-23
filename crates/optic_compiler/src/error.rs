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

    #[snafu(display("Cargo must resolve {key}, got {diagnostics}"))]
    CargoConfiguration {
        key: &'static str,
        diagnostics: String,
    },

    #[snafu(display("Cargo must return {key}, got no matching field"))]
    MissingCargoConfiguration { key: &'static str },

    #[snafu(display("Cargo must report the {key} origin, got {line}"))]
    MissingCargoConfigurationOrigin { key: &'static str, line: String },

    #[snafu(display("Cargo must encode {key} as a string, got {encoded_value}"))]
    InvalidCargoConfigurationValue {
        key: &'static str,
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
