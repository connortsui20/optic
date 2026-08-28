//! Records one successful Cargo invocation.
//!
//! [`BuildRecord`] stores the resolved package and target together with the exact profile, Cargo
//! executable, invocation directory, and ordered argument list. The record does not claim that
//! Cargo invoked rustc or preserve all environment and configuration inputs.

use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::TargetRecord;
use crate::error::InvalidFieldSnafu;
use crate::validation::require_path;
use crate::validation::require_text;

/// The selected inputs for one successful Cargo invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedBuildRecord")]
pub struct BuildRecord {
    package: String,
    package_version: String,
    target: TargetRecord,
    profile: String,
    cargo_program: PathBuf,
    invocation_directory: PathBuf,
    cargo_arguments: Vec<String>,
}

impl BuildRecord {
    /// Creates a record from a successfully resolved Cargo invocation.
    ///
    /// # Errors
    ///
    /// Returns an error if a textual field or program path is empty, the invocation directory is
    /// relative, or no Cargo arguments were recorded.
    pub fn new(
        package: impl Into<String>,
        package_version: impl Into<String>,
        target: TargetRecord,
        profile: impl Into<String>,
        cargo_program: PathBuf,
        invocation_directory: PathBuf,
        cargo_arguments: Vec<String>,
    ) -> Result<Self, Error> {
        let package = package.into();
        let package_version = package_version.into();
        let profile = profile.into();

        require_text("package name", &package)?;
        require_text("package version", &package_version)?;
        require_text("profile", &profile)?;
        require_path("Cargo program", &cargo_program)?;
        require_path("invocation directory", &invocation_directory)?;
        if !invocation_directory.is_absolute() {
            return InvalidFieldSnafu {
                field: "invocation directory",
                actual: format!("a relative path ({})", invocation_directory.display()),
            }
            .fail();
        }
        if cargo_arguments.is_empty() {
            return InvalidFieldSnafu {
                field: "Cargo arguments",
                actual: "an empty list",
            }
            .fail();
        }

        Ok(Self {
            package,
            package_version,
            target,
            profile,
            cargo_program,
            invocation_directory,
            cargo_arguments,
        })
    }

    /// Returns the exact workspace package name.
    pub fn package(&self) -> &str {
        &self.package
    }

    /// Returns the package version resolved by Cargo.
    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    /// Returns the resolved Cargo target.
    pub fn target(&self) -> &TargetRecord {
        &self.target
    }

    /// Returns the explicit Cargo profile name.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Returns the Cargo executable selected by the environment.
    pub fn cargo_program(&self) -> &Path {
        &self.cargo_program
    }

    /// Returns the absolute directory from which Cargo was invoked.
    pub fn invocation_directory(&self) -> &Path {
        &self.invocation_directory
    }

    /// Returns the ordered arguments passed to Cargo.
    pub fn cargo_arguments(&self) -> &[String] {
        &self.cargo_arguments
    }
}

/// The serialized fields that must pass [`BuildRecord`] validation during deserialization.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedBuildRecord {
    package: String,
    package_version: String,
    target: TargetRecord,
    profile: String,
    cargo_program: PathBuf,
    invocation_directory: PathBuf,
    cargo_arguments: Vec<String>,
}

impl TryFrom<UncheckedBuildRecord> for BuildRecord {
    type Error = Error;

    fn try_from(record: UncheckedBuildRecord) -> Result<Self, Self::Error> {
        Self::new(
            record.package,
            record.package_version,
            record.target,
            record.profile,
            record.cargo_program,
            record.invocation_directory,
            record.cargo_arguments,
        )
    }
}
