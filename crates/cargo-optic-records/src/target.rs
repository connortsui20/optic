//! Preserves the exact Cargo target identity selected by a completed build.
//!
//! [`TargetRecord`] keeps the selector class with Cargo's reported name. Downstream code can then
//! use the target without repeating resolution or validation.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::validation::require_text;

/// The Cargo selector class of a resolved target.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CargoTargetKind {
    /// Uses Cargo's `--lib` selector for any library-like metadata kind.
    Lib,

    /// Uses Cargo's `--bin <name>` selector.
    Bin,

    /// Uses Cargo's `--example <name>` selector.
    Example,

    /// Uses Cargo's `--bench <name>` selector.
    Bench,
}

impl fmt::Display for CargoTargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let selector = match self {
            Self::Lib => "lib",
            Self::Bin => "bin",
            Self::Example => "example",
            Self::Bench => "bench",
        };

        formatter.write_str(selector)
    }
}

/// The target identity reported by Cargo metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedTargetRecord")]
pub struct TargetRecord {
    name: String,
    kind: CargoTargetKind,
}

impl TargetRecord {
    /// Creates a target identity from Cargo metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if `name` is empty.
    pub fn new(name: impl Into<String>, kind: CargoTargetKind) -> Result<Self, Error> {
        let name = name.into();

        require_text("target name", &name)?;

        Ok(Self { name, kind })
    }

    /// Returns the exact target name reported by Cargo.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the Cargo selector class for this target.
    pub fn kind(&self) -> CargoTargetKind {
        self.kind
    }
}

/// The serialized fields that must pass [`TargetRecord`] validation during deserialization.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedTargetRecord {
    name: String,
    kind: CargoTargetKind,
}

impl TryFrom<UncheckedTargetRecord> for TargetRecord {
    type Error = Error;

    fn try_from(record: UncheckedTargetRecord) -> Result<Self, Self::Error> {
        Self::new(record.name, record.kind)
    }
}
