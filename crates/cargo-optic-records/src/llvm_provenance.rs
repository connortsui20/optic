//! Retains the selected compiler's effective LLVM configuration once per capture.
//!
//! The capture header owns Rust compiler identity. This record supplies backend and LLVM identity,
//! effective codegen settings, and the recipe used for artifact collection.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::error::InvalidFieldSnafu;
use crate::validation::require_text;

/// The effective link-time optimization (LTO) mode observed by the selected compiler.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlvmLto {
    /// No local or cross-crate LTO.
    Off,
    /// ThinLTO between local regular codegen units.
    LocalThin,
    /// ThinLTO across crates.
    CrossCrateThin,
    /// Fat LTO across crates.
    Fat,
}

/// Effective codegen provenance, independent of the requested Cargo profile name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawLlvmProvenance")]
pub struct LlvmProvenance {
    backend: String,
    llvm_version: Option<String>,
    target: String,
    optimization: String,
    lto: LlvmLto,
    incremental: bool,
    linker_plugin_lto: bool,
    codegen_units: u32,
    recipe_revision: u32,
}

impl LlvmProvenance {
    /// Records the actual backend and settings reported by the compiler.
    ///
    /// LLVM's built-in backend uses `llvm`. Other backend identities remain descriptive text.
    /// `codegen_units` is the configured count, not the number of emitted regular modules.
    ///
    /// # Errors
    ///
    /// Returns an error for empty identity text, an invalid optimization level, a zero codegen-unit
    /// count, or a zero recipe revision.
    // Every setting records an actual compiler observation. A builder with defaults can silently
    // substitute configuration that the selected compiler did not use.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: impl Into<String>,
        llvm_version: Option<String>,
        target: impl Into<String>,
        optimization: impl Into<String>,
        lto: LlvmLto,
        incremental: bool,
        linker_plugin_lto: bool,
        codegen_units: u32,
        recipe_revision: u32,
    ) -> Result<Self, Error> {
        let backend = backend.into();
        let target = target.into();
        let optimization = optimization.into();

        require_text("codegen backend", &backend)?;
        require_text("LLVM effective target", &target)?;

        if let Some(version) = &llvm_version {
            require_text("LLVM version", version)?;
        }

        if !matches!(optimization.as_str(), "0" | "1" | "2" | "3" | "s" | "z") {
            return InvalidFieldSnafu {
                field: "LLVM optimization",
                actual: optimization,
            }
            .fail();
        }

        if codegen_units == 0 || recipe_revision == 0 {
            return InvalidFieldSnafu {
                field: "LLVM provenance",
                actual: "zero codegen units or recipe revision",
            }
            .fail();
        }

        Ok(Self {
            backend,
            llvm_version,
            target,
            optimization,
            lto,
            incremental,
            linker_plugin_lto,
            codegen_units,
            recipe_revision,
        })
    }

    /// Returns the backend identity, with `llvm` for the built-in LLVM backend.
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Returns the LLVM version when the backend has one.
    pub fn llvm_version(&self) -> Option<&str> {
        self.llvm_version.as_deref()
    }

    /// Returns the effective target identity reported by the compiler.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the effective optimization level: `0`, `1`, `2`, `3`, `s`, or `z`.
    pub fn optimization(&self) -> &str {
        &self.optimization
    }

    /// Returns the effective LTO mode.
    pub fn lto(&self) -> LlvmLto {
        self.lto
    }

    /// Returns whether the selected target used incremental compilation.
    pub fn incremental(&self) -> bool {
        self.incremental
    }

    /// Returns whether the selected target delegated LTO to the linker plugin.
    pub fn linker_plugin_lto(&self) -> bool {
        self.linker_plugin_lto
    }

    /// Returns the configured codegen-unit count.
    pub fn codegen_units(&self) -> u32 {
        self.codegen_units
    }

    /// Returns the compiler collection recipe revision.
    pub fn recipe_revision(&self) -> u32 {
        self.recipe_revision
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLlvmProvenance {
    backend: String,
    llvm_version: Option<String>,
    target: String,
    optimization: String,
    lto: LlvmLto,
    incremental: bool,
    linker_plugin_lto: bool,
    codegen_units: u32,
    recipe_revision: u32,
}

impl TryFrom<RawLlvmProvenance> for LlvmProvenance {
    type Error = Error;

    fn try_from(raw: RawLlvmProvenance) -> Result<Self, Error> {
        Self::new(
            raw.backend,
            raw.llvm_version,
            raw.target,
            raw.optimization,
            raw.lto,
            raw.incremental,
            raw.linker_plugin_lto,
            raw.codegen_units,
            raw.recipe_revision,
        )
    }
}
