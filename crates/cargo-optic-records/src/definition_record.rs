//! Identifies the source definition behind a concrete compiler instance.
//!
//! [`DefinitionRecord`] keeps rustc's owning crate and canonical definition path without adding
//! source lookup data. Captured source belongs to a later evidence channel.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::validation::require_text;

/// The definition from which rustc instantiated a function.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "RawDefinitionRecord")]
pub struct DefinitionRecord {
    crate_name: String,
    definition_path: String,
}

impl DefinitionRecord {
    /// Creates a definition identity reported by rustc.
    ///
    /// # Errors
    ///
    /// Returns an error if either name is empty.
    pub fn new(
        crate_name: impl Into<String>,
        definition_path: impl Into<String>,
    ) -> Result<Self, Error> {
        let crate_name = crate_name.into();
        let definition_path = definition_path.into();

        require_text("definition crate name", &crate_name)?;
        require_text("definition path", &definition_path)?;

        Ok(Self {
            crate_name,
            definition_path,
        })
    }

    /// Returns the compiler crate that owns the definition.
    pub fn crate_name(&self) -> &str {
        &self.crate_name
    }

    /// Returns rustc's canonical definition path without concrete generic arguments.
    pub fn definition_path(&self) -> &str {
        &self.definition_path
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefinitionRecord {
    crate_name: String,
    definition_path: String,
}

impl TryFrom<RawDefinitionRecord> for DefinitionRecord {
    type Error = Error;

    fn try_from(definition: RawDefinitionRecord) -> Result<Self, Self::Error> {
        Self::new(definition.crate_name, definition.definition_path)
    }
}
