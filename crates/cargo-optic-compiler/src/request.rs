//! Defines the validated input to one Cargo capture.
//!
//! [`BuildRequest`] separates syntax-level validation from workspace resolution. Construction
//! rejects empty names and uses [`CargoTarget`] to make a single target selection explicit. The
//! request preserves the Cargo feature flags that must apply to metadata and the subsequent build.
//! The compiler crate later resolves the package and target against Cargo metadata, because only a
//! discovered [`crate::Workspace`] can establish whether those names exist.
//!
//! This request represents only a Cargo selection. It does not represent evidence, output,
//! progress, or capture-lifecycle choices.

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

/// A validated Cargo package, target, profile, and feature selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRequest {
    /// Resolves against exact workspace member names.
    package: String,

    /// Cannot represent multiple target selectors.
    target: CargoTarget,

    /// Accepts built-in and custom Cargo profile names.
    profile: String,

    /// Values passed through Cargo's `--features` option.
    features: Vec<String>,

    /// Whether Cargo receives `--all-features`.
    all_features: bool,

    /// Whether Cargo receives `--no-default-features`.
    no_default_features: bool,
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
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
        })
    }

    /// Enables the listed Cargo features.
    ///
    /// # Errors
    ///
    /// Returns an error when an explicitly selected feature name is empty.
    pub fn with_features(mut self, features: Vec<String>) -> Result<Self, InvalidBuildRequest> {
        for feature in &features {
            require_text("feature name", feature)?;
        }
        self.features = features;

        Ok(self)
    }

    /// Enables every feature declared by the selected packages.
    #[must_use]
    pub fn with_all_features(mut self) -> Self {
        self.all_features = true;

        self
    }

    /// Disables the default features of the selected packages.
    #[must_use]
    pub fn without_default_features(mut self) -> Self {
        self.no_default_features = true;

        self
    }

    /// Returns the requested package name.
    pub fn package(&self) -> &str {
        &self.package
    }

    /// Returns the requested Cargo target.
    pub fn target(&self) -> &CargoTarget {
        &self.target
    }

    /// Returns the explicit Cargo profile name.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Returns the values passed through Cargo's `--features` option.
    pub fn features(&self) -> &[String] {
        &self.features
    }

    /// Returns whether Cargo enables every declared feature.
    pub fn all_features(&self) -> bool {
        self.all_features
    }

    /// Returns whether Cargo disables default features.
    pub fn no_default_features(&self) -> bool {
        self.no_default_features
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
    fn rejects_each_empty_request_component() {
        let cases = [
            (
                "",
                CargoTarget::Library,
                "release",
                "package name must not be empty, got an empty string",
            ), // Package name.
            (
                "example",
                CargoTarget::Library,
                "",
                "profile must not be empty, got an empty string",
            ), // Profile name.
            (
                "example",
                CargoTarget::Binary(String::new()),
                "release",
                "binary target name must not be empty, got an empty string",
            ), // Binary target name.
            (
                "example",
                CargoTarget::Example(String::new()),
                "release",
                "example target name must not be empty, got an empty string",
            ), // Example target name.
            (
                "example",
                CargoTarget::Benchmark(String::new()),
                "release",
                "benchmark target name must not be empty, got an empty string",
            ), // Benchmark target name.
        ];

        for (package, target, profile, expected) in cases {
            let error = BuildRequest::new(package, target, profile)
                .expect_err("the empty request component must be rejected");

            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn rejects_empty_feature_names() {
        let request = BuildRequest::new("example", CargoTarget::Library, "release")
            .expect("the fixture request is valid");

        let error = request
            .with_features(vec![String::new()])
            .expect_err("the empty feature name must be rejected");

        assert_eq!(
            error.to_string(),
            "feature name must not be empty, got an empty string"
        );
    }
}
