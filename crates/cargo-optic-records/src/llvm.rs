//! Records complete optimized modules and their exact symbol definitions.
//!
//! Each regular module corresponds to a codegen unit reported by rustc's partition query. A module
//! owns its definitions, and direct aliases name symbols in that same module. Evidence queries
//! resolve alias chains and cycles without using display names or adding another row identity.

use std::collections::HashSet;
use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::ArtifactId;
use crate::ByteRange;
use crate::Error;
use crate::error::InvalidFieldSnafu;
use crate::validation::require_text;

/// The verified optimized artifact stage captured for a regular compiler module.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlvmStage {
    /// Final output after the configured no-LTO LLVM pipeline.
    NoLtoOptimized,
    /// Local ThinLTO output after its pass manager.
    LocalThinLtoPostPassManager,
}

impl fmt::Display for LlvmStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoLtoOptimized => "optimized, no LTO",
            Self::LocalThinLtoPostPassManager => "optimized, local ThinLTO after pass manager",
        })
    }
}

/// The exact top-level LLVM construct associated with a symbol and range.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LlvmDefinitionKind {
    /// A standalone function definition, including its body.
    Function,
    /// A direct alias to an exact symbol in the same module.
    DirectAlias {
        /// The decoded raw target symbol, without the LLVM `@` prefix.
        target: String,
    },
    /// An expression alias that this release cannot resolve exactly.
    ExpressionAlias,
    /// An indirect function that this release cannot resolve exactly.
    Ifunc,
}

/// An exact raw symbol and its unchanged text range in the owning module.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawLlvmDefinitionRecord")]
pub struct LlvmDefinitionRecord {
    raw_symbol: String,
    range: ByteRange,
    kind: LlvmDefinitionKind,
}

impl LlvmDefinitionRecord {
    /// Records a function or alias range without resolving alias targets.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty raw symbol or direct alias target.
    pub fn new(
        raw_symbol: impl Into<String>,
        range: ByteRange,
        kind: LlvmDefinitionKind,
    ) -> Result<Self, Error> {
        let raw_symbol = raw_symbol.into();

        require_text("LLVM raw symbol", &raw_symbol)?;

        if let LlvmDefinitionKind::DirectAlias { target } = &kind {
            require_text("LLVM alias target", target)?;
        }

        Ok(Self {
            raw_symbol,
            range,
            kind,
        })
    }

    /// Returns the exact decoded symbol without the LLVM `@` prefix.
    pub fn raw_symbol(&self) -> &str {
        &self.raw_symbol
    }

    /// Returns the unchanged definition or alias text range.
    pub fn range(&self) -> ByteRange {
        self.range
    }

    /// Returns the construct kind and direct alias target when present.
    pub fn kind(&self) -> &LlvmDefinitionKind {
        &self.kind
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLlvmDefinitionRecord {
    raw_symbol: String,
    range: ByteRange,
    kind: LlvmDefinitionKind,
}

impl TryFrom<RawLlvmDefinitionRecord> for LlvmDefinitionRecord {
    type Error = Error;

    fn try_from(raw: RawLlvmDefinitionRecord) -> Result<Self, Error> {
        Self::new(raw.raw_symbol, raw.range, raw.kind)
    }
}

/// One complete regular compiler module and its indexed top-level definitions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawLlvmModuleRecord")]
pub struct LlvmModuleRecord {
    artifact: ArtifactId,
    compiler_module: String,
    stage: LlvmStage,
    definitions: Vec<LlvmDefinitionRecord>,
}

impl LlvmModuleRecord {
    /// Associates a complete module with its exact definitions. Empty definition lists are valid.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty module identity or duplicate raw symbols within the module.
    pub fn new(
        artifact: ArtifactId,
        compiler_module: impl Into<String>,
        stage: LlvmStage,
        definitions: Vec<LlvmDefinitionRecord>,
    ) -> Result<Self, Error> {
        let compiler_module = compiler_module.into();

        require_text("LLVM compiler module", &compiler_module)?;

        let mut raw_symbols = HashSet::with_capacity(definitions.len());

        for definition in &definitions {
            if !raw_symbols.insert(definition.raw_symbol()) {
                return InvalidFieldSnafu {
                    field: "LLVM module symbols",
                    actual: format!("duplicate {}", definition.raw_symbol()),
                }
                .fail();
            }
        }

        Ok(Self {
            artifact,
            compiler_module,
            stage,
            definitions,
        })
    }

    /// Returns the artifact containing the complete textual module.
    pub fn artifact(&self) -> ArtifactId {
        self.artifact
    }

    /// Returns the regular codegen-unit module identity supplied by the compiler.
    pub fn compiler_module(&self) -> &str {
        &self.compiler_module
    }

    /// Returns the verified optimized collection stage.
    pub fn stage(&self) -> LlvmStage {
        self.stage
    }

    /// Returns definitions in their recorded order, without resolving aliases.
    pub fn definitions(&self) -> &[LlvmDefinitionRecord] {
        &self.definitions
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLlvmModuleRecord {
    artifact: ArtifactId,
    compiler_module: String,
    stage: LlvmStage,
    definitions: Vec<LlvmDefinitionRecord>,
}

impl TryFrom<RawLlvmModuleRecord> for LlvmModuleRecord {
    type Error = Error;

    fn try_from(raw: RawLlvmModuleRecord) -> Result<Self, Error> {
        Self::new(
            raw.artifact,
            raw.compiler_module,
            raw.stage,
            raw.definitions,
        )
    }
}

/// Complete optimized modules or a durable unsupported-configuration reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlvmCollection {
    /// Every expected regular module from a supported compiler configuration.
    Collected(Vec<LlvmModuleRecord>),
    /// Collection was excluded before any extra LLVM artifact request.
    NotCaptured(UnsupportedLlvmConfiguration),
}

/// A configuration for which this release cannot claim exact optimized LLVM.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedLlvmConfiguration {
    /// The selected target uses incremental compilation.
    Incremental,

    /// The selected target uses cross-crate ThinLTO.
    CrossCrateThinLto,

    /// The selected target uses fat LTO.
    FatLto,

    /// The selected target delegates LTO to the linker plugin.
    LinkerPluginLto,

    /// The selected target uses another compiler backend.
    OtherBackend,

    /// The compiler release has no verified optimized-artifact stage mapping.
    UnverifiedCompiler,
}

impl fmt::Display for UnsupportedLlvmConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Incremental => "optimized LLVM is unavailable for incremental compilation",
            Self::CrossCrateThinLto => "optimized LLVM is unavailable for cross-crate ThinLTO",
            Self::FatLto => "optimized LLVM is unavailable for fat LTO",
            Self::LinkerPluginLto => "optimized LLVM is unavailable for linker-plugin LTO",
            Self::OtherBackend => "optimized LLVM is unavailable for this compiler backend",
            Self::UnverifiedCompiler => {
                "optimized LLVM artifact stages are not verified for this compiler release"
            }
        })
    }
}
