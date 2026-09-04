//! Records one concrete function instance selected by rustc.
//!
//! [`InstanceRecord`] joins a definition, concrete display name, raw symbol, and nonempty set of
//! codegen-unit placements. The raw symbol is compiler-generated evidence; the display name is for
//! search and output only.

use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;

use crate::DefinitionRecord;
use crate::Error;
use crate::PlacementRecord;
use crate::error::InvalidFieldSnafu;
use crate::validation::require_text;

/// One concrete function instance selected for code generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawInstanceRecord")]
pub struct InstanceRecord {
    definition: DefinitionRecord,
    display_name: String,
    raw_symbol: String,
    placements: Vec<PlacementRecord>,
}

impl InstanceRecord {
    /// Creates a concrete instance and its codegen-unit placements.
    ///
    /// # Errors
    ///
    /// Returns an error if a name is empty, no placements exist, or a codegen unit occurs more than
    /// once.
    pub fn new(
        definition: DefinitionRecord,
        display_name: impl Into<String>,
        raw_symbol: impl Into<String>,
        placements: Vec<PlacementRecord>,
    ) -> Result<Self, Error> {
        let display_name = display_name.into();
        let raw_symbol = raw_symbol.into();

        require_text("instance display name", &display_name)?;
        require_text("instance raw symbol", &raw_symbol)?;
        if placements.is_empty() {
            return InvalidFieldSnafu {
                field: "instance placements",
                actual: "an empty list",
            }
            .fail();
        }

        let mut codegen_units = HashSet::with_capacity(placements.len());
        for placement in &placements {
            if !codegen_units.insert(placement.codegen_unit()) {
                return InvalidFieldSnafu {
                    field: "instance placements",
                    actual: format!("a duplicate codegen unit ({})", placement.codegen_unit()),
                }
                .fail();
            }
        }

        Ok(Self {
            definition,
            display_name,
            raw_symbol,
            placements,
        })
    }

    /// Returns the definition that rustc instantiated.
    pub fn definition(&self) -> &DefinitionRecord {
        &self.definition
    }

    /// Returns the concrete Rust display name, including generic arguments.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the exact symbol name that rustc assigned to this instance.
    pub fn raw_symbol(&self) -> &str {
        &self.raw_symbol
    }

    /// Returns every codegen-unit placement for this exact instance.
    pub fn placements(&self) -> &[PlacementRecord] {
        &self.placements
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstanceRecord {
    definition: DefinitionRecord,
    display_name: String,
    raw_symbol: String,
    placements: Vec<PlacementRecord>,
}

impl TryFrom<RawInstanceRecord> for InstanceRecord {
    type Error = Error;

    fn try_from(instance: RawInstanceRecord) -> Result<Self, Self::Error> {
        Self::new(
            instance.definition,
            instance.display_name,
            instance.raw_symbol,
            instance.placements,
        )
    }
}
