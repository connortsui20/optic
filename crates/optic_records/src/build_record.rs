//! Records the reproducible portion of one completed Cargo invocation.
//!
//! [`BuildRecord`] stores the resolved package and target together with the exact profile, Cargo
//! executable, and ordered argument list. The compiler crate constructs it only after Cargo exits
//! successfully; durable readers obtain the same guarantees through validated deserialization.

use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::InvalidFieldSnafu;
use crate::TargetRecord;
use crate::validation::require_path;
use crate::validation::require_text;

/// The reproducible portion of the Cargo invocation for one capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedBuildRecord")]
pub struct BuildRecord {
    package: String,
    package_version: String,
    target: TargetRecord,
    profile: String,
    cargo_program: PathBuf,
    cargo_arguments: Vec<String>,
}

impl BuildRecord {
    /// Creates a record from a successfully resolved Cargo invocation.
    ///
    /// # Errors
    ///
    /// Returns an error if a textual field or program path is empty, or if no Cargo arguments were
    /// recorded.
    pub fn new(
        package: impl Into<String>,
        package_version: impl Into<String>,
        target: TargetRecord,
        profile: impl Into<String>,
        cargo_program: PathBuf,
        cargo_arguments: Vec<String>,
    ) -> Result<Self, Error> {
        let package = package.into();
        let package_version = package_version.into();
        let profile = profile.into();

        require_text("package name", &package)?;
        require_text("package version", &package_version)?;
        require_text("profile", &profile)?;
        require_path("Cargo program", &cargo_program)?;
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
            cargo_arguments,
        })
    }

    /// Returns the exact workspace package name.
    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    /// Returns the package version resolved by Cargo.
    #[must_use]
    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    /// Returns the resolved Cargo target.
    #[must_use]
    pub fn target(&self) -> &TargetRecord {
        &self.target
    }

    /// Returns the explicit Cargo profile name.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Returns the Cargo executable selected by the environment.
    #[must_use]
    pub fn cargo_program(&self) -> &Path {
        &self.cargo_program
    }

    /// Returns the ordered arguments passed to Cargo.
    #[must_use]
    pub fn cargo_arguments(&self) -> &[String] {
        &self.cargo_arguments
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedBuildRecord {
    package: String,
    package_version: String,
    target: TargetRecord,
    profile: String,
    cargo_program: PathBuf,
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
            record.cargo_arguments,
        )
    }
}
