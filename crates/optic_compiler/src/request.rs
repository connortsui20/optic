//! Defines the validated input to one Cargo capture.
//!
//! [`BuildRequest`] separates syntax-level validation from workspace resolution. Construction
//! rejects empty names and uses [`CargoTarget`] to make a single target selection explicit. The
//! compiler crate later resolves the package and target against Cargo metadata, because only a
//! discovered [`crate::Workspace`] can establish whether those names exist.

use std::fmt;

use optic_records::CargoTargetKind;
use snafu::Snafu;

/// Selects exactly one target from a Cargo package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CargoTarget {
    /// Selects the package's library-like target.
    Library,

    /// Selects a binary by its Cargo metadata name.
    Binary(String),

    /// Selects an example by its Cargo metadata name.
    Example(String),

    /// Selects a benchmark by its Cargo metadata name.
    Benchmark(String),
}

impl CargoTarget {
    pub(crate) fn kind(&self) -> CargoTargetKind {
        match self {
            Self::Library => CargoTargetKind::Lib,
            Self::Binary(_) => CargoTargetKind::Bin,
            Self::Example(_) => CargoTargetKind::Example,
            Self::Benchmark(_) => CargoTargetKind::Bench,
        }
    }

    pub(crate) fn selector_arguments(&self) -> Vec<String> {
        match self {
            Self::Library => vec!["--lib".to_owned()],
            Self::Binary(name) => vec!["--bin".to_owned(), name.clone()],
            Self::Example(name) => vec!["--example".to_owned(), name.clone()],
            Self::Benchmark(name) => vec!["--bench".to_owned(), name.clone()],
        }
    }
}

impl fmt::Display for CargoTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Library => formatter.write_str("library"),
            Self::Binary(name) => write!(formatter, "binary {name}"),
            Self::Example(name) => write!(formatter, "example {name}"),
            Self::Benchmark(name) => write!(formatter, "benchmark {name}"),
        }
    }
}

/// A validated request for one Cargo package, target, and profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRequest {
    /// Resolves against exact workspace member names.
    package: String,

    /// Cannot represent multiple target selectors.
    target: CargoTarget,

    /// Accepts built-in and custom Cargo profile names.
    profile: String,
}

impl BuildRequest {
    /// Creates one explicit Cargo build request.
    ///
    /// # Errors
    ///
    /// Returns an error when the package, profile, or named target is empty.
    pub fn new(
        package: impl Into<String>,
        target: CargoTarget,
        profile: impl Into<String>,
    ) -> Result<Self, InvalidBuildRequest> {
        let package = package.into();
        let profile = profile.into();

        require_text("package name", &package)?;
        require_text("profile", &profile)?;

        match &target {
            CargoTarget::Library => {}
            CargoTarget::Binary(name) => require_text("binary target name", name)?,
            CargoTarget::Example(name) => require_text("example target name", name)?,
            CargoTarget::Benchmark(name) => require_text("benchmark target name", name)?,
        }

        Ok(Self {
            package,
            target,
            profile,
        })
    }

    /// Returns the requested package name.
    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    /// Returns the requested Cargo target.
    #[must_use]
    pub fn target(&self) -> &CargoTarget {
        &self.target
    }

    /// Returns the explicit Cargo profile name.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }
}

fn require_text(field: &'static str, value: &str) -> Result<(), InvalidBuildRequest> {
    if value.is_empty() {
        return InvalidBuildRequestSnafu { field }.fail();
    }

    Ok(())
}

/// Identifies the empty component of a syntactically invalid build request.
#[derive(Debug, Snafu)]
#[snafu(display("{field} must not be empty, got an empty string"))]
pub struct InvalidBuildRequest {
    field: &'static str,
}

#[cfg(test)]
mod tests {
    use super::BuildRequest;
    use super::CargoTarget;

    #[test]
    fn rejects_empty_package_profile_and_named_targets() {
        assert!(BuildRequest::new("", CargoTarget::Library, "release").is_err());
        assert!(BuildRequest::new("example", CargoTarget::Library, "").is_err());
        assert!(
            BuildRequest::new("example", CargoTarget::Binary(String::new()), "release").is_err()
        );
        assert!(
            BuildRequest::new("example", CargoTarget::Example(String::new()), "release").is_err()
        );
        assert!(
            BuildRequest::new("example", CargoTarget::Benchmark(String::new()), "release",)
                .is_err()
        );
    }
}
