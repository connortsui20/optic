//! Retains Cargo's selected-target observation for freshness comparison.
//!
//! This record uses Cargo metadata's artifact schema. Normalization removes observation order and
//! the transient freshness flag, while preserving Cargo's actual profile settings.

use cargo_metadata::Artifact;
use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::error::InvalidFieldSnafu;
use crate::validation::require_absolute_normalized_path;
use crate::validation::require_text;

/// A validated Cargo artifact with sorted unordered fields and `fresh` cleared.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "Artifact")]
pub struct CargoArtifactRecord(Artifact);

impl CargoArtifactRecord {
    /// Normalizes the artifact for storage and comparison.
    ///
    /// # Errors
    ///
    /// Returns an error for incomplete package or target identity, an empty optimization level, or
    /// source, manifest, and output paths that are not absolute and lexically normalized.
    /// Empty features and output filenames are valid. Referenced files need not still exist.
    pub fn new(mut artifact: Artifact) -> Result<Self, Error> {
        require_text("Cargo artifact package ID", &artifact.package_id.repr)?;
        require_text("Cargo artifact target name", &artifact.target.name)?;
        require_text(
            "Cargo artifact optimization level",
            &artifact.profile.opt_level,
        )?;

        if artifact.target.kind.is_empty() || artifact.target.crate_types.is_empty() {
            return InvalidFieldSnafu {
                field: "Cargo artifact target identity",
                actual: "empty target kinds or crate types",
            }
            .fail();
        }

        for kind in &artifact.target.kind {
            require_text("Cargo artifact target kind", &kind.to_string())?;
        }

        for kind in &artifact.target.crate_types {
            require_text("Cargo artifact crate type", &kind.to_string())?;
        }

        for feature in artifact
            .features
            .iter()
            .chain(&artifact.target.required_features)
        {
            require_text("Cargo artifact feature", feature)?;
        }

        require_absolute_normalized_path(
            "Cargo artifact manifest path",
            artifact.manifest_path.as_std_path(),
        )?;
        require_absolute_normalized_path(
            "Cargo artifact source path",
            artifact.target.src_path.as_std_path(),
        )?;

        for path in artifact.filenames.iter().chain(artifact.executable.iter()) {
            require_absolute_normalized_path("Cargo artifact output path", path.as_std_path())?;
        }

        artifact.fresh = false;
        artifact.features.sort();
        artifact.filenames.sort();
        artifact.target.kind.sort();
        artifact.target.crate_types.sort();
        artifact.target.required_features.sort();

        Ok(Self(artifact))
    }

    /// Returns the immutable normalized Cargo observation.
    pub fn artifact(&self) -> &Artifact {
        &self.0
    }
}

impl TryFrom<Artifact> for CargoArtifactRecord {
    type Error = Error;

    fn try_from(artifact: Artifact) -> Result<Self, Self::Error> {
        Self::new(artifact)
    }
}
