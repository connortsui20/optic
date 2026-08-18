//! Public requests and views exposed by the Optic application.
//!
//! These types contain product concepts rather than database rows or compiler artifact paths. Both
//! the CLI and future terminal interface consume the same views.

use std::fmt;
use std::path::PathBuf;

use cargo_ir::CargoTarget;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{CaptureId, InstanceId};

/// Controls whether a capture can use matching persistent evidence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CachePolicy {
    /// Uses a matching completed capture when one exists.
    #[default]
    Reuse,

    /// Runs the compiler and publishes a new capture.
    Refresh,
}

/// One compiler output that an inspection command can show.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CompilerOutput {
    /// Optimized LLVM IR from the saved code-generation unit.
    #[default]
    #[value(name = "llvm")]
    Llvm,

    /// LLVM IR before the LLVM optimization pipeline.
    #[value(name = "llvm-pre-opt")]
    LlvmPreOpt,
}

impl CompilerOutput {
    pub(crate) const fn stage(self) -> &'static str {
        match self {
            Self::Llvm => "llvm-optimized",
            Self::LlvmPreOpt => "llvm-pre-optimization",
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Llvm => "llvm",
            Self::LlvmPreOpt => "llvm-pre-opt",
        }
    }

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Llvm => "LLVM (optimized)",
            Self::LlvmPreOpt => "LLVM (before optimization)",
        }
    }
}

impl fmt::Display for CompilerOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A normalized Cargo target selection for an enriched capture.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuildSpec {
    /// An optional absolute manifest path.
    pub manifest_path: Option<PathBuf>,

    /// An optional selected package.
    pub package: Option<String>,

    /// An optional selected library, binary, benchmark, or example.
    pub target: Option<CargoTarget>,

    /// An optional Cargo profile.
    pub profile: Option<String>,

    /// Enabled Cargo features.
    pub features: Vec<String>,

    /// Whether every Cargo feature is enabled.
    pub all_features: bool,

    /// Whether default Cargo features are disabled.
    pub no_default_features: bool,

    /// An optional compiler target triple.
    pub target_triple: Option<String>,

    /// Whether Cargo must use the lock file without changing it.
    pub locked: bool,

    /// Whether Cargo must avoid network access.
    pub offline: bool,

    /// Whether Cargo must use both locked and offline behavior.
    pub frozen: bool,
}

/// A completed or reused compiler-evidence capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaptureSummary {
    /// The immutable capture identifier.
    pub id: CaptureId,

    /// Unix time in milliseconds when the capture was published.
    pub created_at_ms: u64,

    /// Whether this request reused an existing completed capture.
    pub reused: bool,

    /// The exact rustc release.
    pub rustc_release: String,

    /// The embedded LLVM version.
    pub llvm_version: String,

    /// The selected compiler target or host.
    pub target: String,

    /// The number of concrete mono items recorded by rustc.
    pub instance_count: usize,
}

/// One concrete compiler instance returned by lookup.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstanceSummary {
    /// The opaque instance identifier.
    pub id: InstanceId,

    /// The source-level Rust definition path.
    pub definition: String,

    /// The concrete Rust display name, including generic arguments.
    pub display_name: String,

    /// Whether at least one supported LLVM stage contains a standalone body.
    pub has_body: bool,
}

/// The candidates returned by one definition lookup.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FindResult {
    /// The queried capture.
    pub capture_id: CaptureId,

    /// Every exact or fallback substring candidate.
    pub instances: Vec<InstanceSummary>,
}

/// One exact LLVM function body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BodyView {
    /// The captured compiler stage.
    pub stage: String,

    /// The compiler-owned module name.
    pub module: String,

    /// The raw LLVM symbol.
    pub symbol: String,

    /// The exact textual LLVM definition bytes.
    pub text: String,
}

/// Captured source for one definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceView {
    /// The original source path.
    pub path: String,

    /// The one-based line on which the displayed item starts.
    pub start_line: usize,

    /// The captured item text.
    pub text: String,
}

/// The complete inspection view for one concrete instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShowView {
    /// The queried capture.
    pub capture_id: CaptureId,

    /// The concrete instance.
    pub instance: InstanceSummary,

    /// The selected compiler output.
    pub output: CompilerOutput,

    /// Every standalone body for the selected compiler output.
    pub bodies: Vec<BodyView>,

    /// Captured source when the caller requested it and Optic found one item.
    pub source: Option<SourceView>,
}
