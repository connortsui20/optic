//! Describes one Cargo analysis request.
//!
//! [`BuildRequest`] contains the target selection and evidence policy that `cargo rustc`
//! understands. The caller owns persistence and supplies an empty analysis directory for rustc
//! temporaries.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One Cargo target kind and optional target name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "name")]
pub enum CargoTarget {
    /// The package library target.
    Library,

    /// A named binary target.
    Binary(String),

    /// A named benchmark target.
    Benchmark(String),

    /// A named example target.
    Example(String),
}

/// The compiler changes that an evidence capture may make.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum CaptureProfile {
    /// Preserve the selected target's code generation settings.
    ///
    /// Optic still saves compiler temporaries so it can collect evidence. It does not change
    /// linking, symbol mangling, debug information, LTO, codegen units, or the panic strategy.
    #[default]
    Faithful,

    /// Add source-oriented names and line tables to the captured evidence.
    Enriched,

    /// Apply explicit compiler arguments supplied by the caller.
    Experiment {
        /// Additional arguments passed to rustc after Optic's evidence arguments.
        rustc_arguments: Vec<String>,
    },
}

/// A normalized request for one Cargo analysis.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuildRequest {
    /// The Cargo workspace in which the command runs.
    pub workspace_root: PathBuf,

    /// An optional manifest path passed to Cargo.
    pub manifest_path: Option<PathBuf>,

    /// An optional selected package.
    pub package: Option<String>,

    /// An optional selected Cargo target.
    pub target: Option<CargoTarget>,

    /// An optional Cargo profile name.
    pub profile: Option<String>,

    /// Enabled Cargo features.
    pub features: Vec<String>,

    /// Whether Cargo enables every declared feature.
    pub all_features: bool,

    /// Whether Cargo disables default features.
    pub no_default_features: bool,

    /// An optional compiler target triple.
    pub target_triple: Option<String>,

    /// Whether Cargo must use the lock file without changing it.
    pub locked: bool,

    /// Whether Cargo must avoid network access.
    pub offline: bool,

    /// Whether Cargo must use both locked and offline behavior.
    pub frozen: bool,

    /// The permitted compiler changes for this capture.
    pub capture_profile: CaptureProfile,

    /// The directory in which rustc writes mutable analysis artifacts.
    ///
    /// This directory **must** be empty when the capture starts.
    pub analysis_directory: PathBuf,
}
