//! Records one codegen-unit placement for a concrete instance.
//!
//! [`PlacementRecord`] preserves rustc's placement attributes. Linkage and visibility remain text
//! because rustc's internal enums have no stability contract and must not become part of this
//! public API. A placement does not prove that the instance retains a standalone emitted body after
//! code generation or optimization.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::validation::require_text;

/// One placement of an instance in a rustc codegen unit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawPlacementRecord")]
pub struct PlacementRecord {
    codegen_unit: String,
    linkage: String,
    visibility: String,
    local_copy: bool,
    size_estimate: u64,
}

impl PlacementRecord {
    /// Creates a codegen-unit placement reported by rustc.
    ///
    /// # Errors
    ///
    /// Returns an error if the codegen-unit name, linkage, or visibility is empty.
    pub fn new(
        codegen_unit: impl Into<String>,
        linkage: impl Into<String>,
        visibility: impl Into<String>,
        local_copy: bool,
        size_estimate: u64,
    ) -> Result<Self, Error> {
        let codegen_unit = codegen_unit.into();
        let linkage = linkage.into();
        let visibility = visibility.into();

        require_text("codegen unit", &codegen_unit)?;
        require_text("placement linkage", &linkage)?;
        require_text("placement visibility", &visibility)?;

        Ok(Self {
            codegen_unit,
            linkage,
            visibility,
            local_copy,
            size_estimate,
        })
    }

    /// Returns the exact codegen-unit name reported by rustc.
    pub fn codegen_unit(&self) -> &str {
        &self.codegen_unit
    }

    /// Returns rustc's linkage name for this placement.
    pub fn linkage(&self) -> &str {
        &self.linkage
    }

    /// Returns rustc's visibility name for this placement.
    pub fn visibility(&self) -> &str {
        &self.visibility
    }

    /// Returns whether rustc placed a local copy instead of a globally shared instance.
    pub fn local_copy(&self) -> bool {
        self.local_copy
    }

    /// Returns rustc's estimated instance size before code generation.
    pub fn size_estimate(&self) -> u64 {
        self.size_estimate
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlacementRecord {
    codegen_unit: String,
    linkage: String,
    visibility: String,
    local_copy: bool,
    size_estimate: u64,
}

impl TryFrom<RawPlacementRecord> for PlacementRecord {
    type Error = Error;

    fn try_from(placement: RawPlacementRecord) -> Result<Self, Self::Error> {
        Self::new(
            placement.codegen_unit,
            placement.linkage,
            placement.visibility,
            placement.local_copy,
            placement.size_estimate,
        )
    }
}
