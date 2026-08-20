//! Public requests and views exposed by the Optic application.
//!
//! These types contain product concepts rather than database rows or compiler artifact paths. Both
//! the CLI and future terminal interface consume the same views.

use std::fmt;
use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{CallSiteDelta, CallSiteSummary, CaptureId, InstanceId, LlvmStage};

/// One Cargo target selected through the product API.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "name")]
pub enum BuildTarget {
    /// The package library target.
    Library,

    /// A named binary target.
    Binary(String),

    /// A named benchmark target.
    Benchmark(String),

    /// A named example target.
    Example(String),
}

/// Controls which compiler changes Optic can make while it records evidence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureProfile {
    /// Preserves the selected target's code-generation settings.
    #[default]
    Faithful,

    /// Adds source-oriented diagnostic evidence such as line tables.
    Enriched,

    /// Applies explicit compiler arguments supplied with the build specification.
    Experiment,
}

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
    pub(crate) const fn stage(self) -> LlvmStage {
        match self {
            Self::Llvm => LlvmStage::Optimized,
            Self::LlvmPreOpt => LlvmStage::PreOptimization,
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
    pub target: Option<BuildTarget>,

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

    /// The permitted compiler changes for this capture.
    pub capture_profile: CaptureProfile,

    /// Additional rustc arguments for an experiment capture.
    pub rustc_arguments: Vec<String>,
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

    /// The evidence profile used for this capture.
    pub capture_profile: CaptureProfile,

    /// The number of concrete compiler instances recorded by rustc.
    pub instance_count: usize,

    /// The number of captured LLVM artifacts.
    pub module_count: usize,
}

/// One subprocess command recorded for a capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandView {
    /// The executable passed to the operating system.
    pub program: String,

    /// The ordered command arguments.
    pub arguments: Vec<String>,
}

/// One compiler-related environment variable recorded for a capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentView {
    /// The environment variable name.
    pub name: String,

    /// The recorded value.
    pub value: String,
}

/// Provenance and record counts for one compiler-owned LLVM artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactSummary {
    /// The compiler-owned artifact name.
    pub name: String,

    /// The normalized stage, when Optic understands it.
    pub stage: Option<LlvmStage>,

    /// The exact compiler stage suffix.
    pub compiler_stage: String,

    /// The inferred codegen-unit name.
    pub codegen_unit: Option<String>,

    /// The recorded LTO scope.
    pub lto: String,

    /// The capture mechanism.
    pub capture_method: String,

    /// Indexed function definitions.
    pub definitions: usize,

    /// Indexed function declarations.
    pub declarations: usize,

    /// Indexed LLVM aliases.
    pub aliases: usize,
}

/// Reproducibility details for one immutable capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaptureDetails {
    /// The standard capture summary.
    pub summary: CaptureSummary,

    /// The normalized product request.
    pub request: BuildSpec,

    /// The exact Cargo subprocess command.
    pub cargo: CommandView,

    /// The selected rustc command, when rustc ran.
    pub rustc: Option<CommandView>,

    /// The effective wrapper chain, from outermost to innermost.
    pub wrapper_chain: Vec<String>,

    /// Codegen-related environment inherited by Cargo.
    pub environment: Vec<EnvironmentView>,

    /// Compiler arguments injected by Optic.
    pub injected_rustc_arguments: Vec<String>,

    /// Every captured LLVM artifact and its exact stage provenance.
    pub artifacts: Vec<ArtifactSummary>,
}

/// The result of removing stored Optic evidence for one workspace.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanSummary {
    /// The evidence-store path selected from Cargo metadata.
    pub path: PathBuf,

    /// Whether current or legacy evidence existed and was removed.
    pub removed: bool,
}

/// Workspace evidence-store size and object counts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoreStatus {
    /// Completed captures in the catalog.
    pub captures: usize,

    /// Content-addressed blob files on disk.
    pub blobs: usize,

    /// Total blob bytes on disk.
    pub blob_bytes: u64,
}

/// The result of removing one immutable capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoveSummary {
    /// The full removed capture ID.
    pub capture_id: CaptureId,
}

/// The result of deleting blobs that no capture references.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GcSummary {
    /// Unreferenced blob files removed.
    pub removed_blobs: usize,

    /// Unreferenced blob bytes removed.
    pub removed_bytes: u64,
}

/// The result of verifying all blobs referenced by completed captures.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerifySummary {
    /// Distinct referenced blobs whose digest was verified.
    pub verified_blobs: usize,

    /// Referenced bytes whose digest was verified.
    pub verified_bytes: u64,
}

/// One concrete compiler instance returned by lookup.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstanceSummary {
    /// The opaque instance identifier.
    pub id: InstanceId,

    /// The compiler crate that owns the source definition.
    pub crate_name: String,

    /// The source-level Rust definition path.
    pub definition: String,

    /// The concrete Rust display name, including generic arguments.
    pub display_name: String,

    /// The exact source range reported by rustc, when available.
    pub source: Option<SourceLocation>,

    /// Standalone evidence available at each supported compiler output.
    pub availability: Vec<OutputAvailability>,
}

/// A compiler-owned source range for one definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLocation {
    /// The compiler source filename after rustc path remapping.
    pub path: String,

    /// The inclusive byte offset in that file.
    pub byte_start: u64,

    /// The exclusive byte offset in that file.
    pub byte_end: u64,

    /// The one-based starting line.
    pub line_start: usize,

    /// The zero-based starting character column.
    pub column_start: usize,

    /// The one-based ending line.
    pub line_end: usize,

    /// The zero-based ending character column.
    pub column_end: usize,
}

/// Standalone symbol records available for one compiler output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutputAvailability {
    /// The compiler output to which the counts apply.
    pub output: CompilerOutput,

    /// Exact function definitions for this instance's raw symbol.
    pub definitions: usize,

    /// Exact declarations for this instance's raw symbol.
    pub declarations: usize,

    /// Exact aliases for this instance's raw symbol.
    pub aliases: usize,
}

impl OutputAvailability {
    /// Whether this stage contains a standalone function body.
    #[must_use]
    pub const fn has_definition(&self) -> bool {
        self.definitions != 0
    }
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
    pub stage: LlvmStage,

    /// The compiler-owned module name.
    pub module: String,

    /// The raw LLVM symbol.
    pub symbol: String,

    /// The exact textual LLVM definition bytes.
    pub text: String,

    /// A compact structural summary of the displayed LLVM body.
    pub summary: LlvmBodySummary,
}

/// A compact, deterministic summary of one textual LLVM function body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmBodySummary {
    /// The exact UTF-8 byte length of the body.
    pub bytes: usize,

    /// The number of instruction-like LLVM lines.
    pub instructions: usize,

    /// Lines that contain a fixed-width LLVM vector type.
    pub vector_lines: usize,

    /// Distinct fixed vector lane counts in ascending order.
    pub vector_widths: Vec<usize>,

    /// Structural classification of `call`, `invoke`, and `callbr` instructions.
    pub call_sites: CallSiteSummary,

    /// References to bounds-check or panic runtime symbols.
    pub safety_checks: usize,
}

impl LlvmBodySummary {
    pub(crate) fn from_text(text: &str) -> Self {
        let mut summary = Self {
            bytes: text.len(),
            instructions: 0,
            vector_lines: 0,
            vector_widths: Vec::new(),
            call_sites: CallSiteSummary::default(),
            safety_checks: 0,
        };

        for line in text.lines().map(str::trim) {
            let is_call_site = summary.call_sites.record_line(line);
            if line.starts_with('%')
                || line.starts_with("store ")
                || is_call_site
                || line.starts_with("ret ")
                || line.starts_with("br ")
                || line.starts_with("switch ")
                || line.starts_with("unreachable")
            {
                summary.instructions += 1;
            }
            if let Some(width) = vector_width(line) {
                summary.vector_lines += 1;
                summary.vector_widths.push(width);
            }
            if line.contains("panic_bounds_check")
                || line.contains("slice_index_fail")
                || line.contains("begin_panic")
            {
                summary.safety_checks += 1;
            }
        }
        summary.vector_widths.sort_unstable();
        summary.vector_widths.dedup();

        summary
    }
}

fn vector_width(line: &str) -> Option<usize> {
    let (_, suffix) = line.split_once('<')?;
    let (width, _) = suffix.split_once(" x ")?;

    width.parse().ok()
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

/// Aggregated structure for every standalone body selected for one instance and stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BodySetSummary {
    /// Standalone bodies included in the aggregate.
    pub bodies: usize,

    /// Total textual LLVM bytes.
    pub bytes: usize,

    /// Total instruction-like LLVM lines.
    pub instructions: usize,

    /// Total lines that contain a fixed-width vector type.
    pub vector_lines: usize,

    /// Distinct fixed vector lane counts in ascending order.
    pub vector_widths: Vec<usize>,

    /// Structural classification of `call`, `invoke`, and `callbr` instructions.
    pub call_sites: CallSiteSummary,

    /// Total references to bounds-check or panic runtime symbols.
    pub safety_checks: usize,
}

impl BodySetSummary {
    pub(crate) fn from_bodies(bodies: &[BodyView]) -> Self {
        let mut summary = Self {
            bodies: bodies.len(),
            bytes: 0,
            instructions: 0,
            vector_lines: 0,
            vector_widths: Vec::new(),
            call_sites: CallSiteSummary::default(),
            safety_checks: 0,
        };
        for body in bodies {
            summary.bytes += body.summary.bytes;
            summary.instructions += body.summary.instructions;
            summary.vector_lines += body.summary.vector_lines;
            summary
                .vector_widths
                .extend(body.summary.vector_widths.iter().copied());
            summary.call_sites.add_assign(&body.summary.call_sites);
            summary.safety_checks += body.summary.safety_checks;
        }
        summary.vector_widths.sort_unstable();
        summary.vector_widths.dedup();

        summary
    }
}

/// Signed structural changes from the first compared body set to the second.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BodySetDelta {
    /// Change in standalone body count.
    pub bodies: i128,

    /// Change in textual LLVM bytes.
    pub bytes: i128,

    /// Change in instruction-like LLVM lines.
    pub instructions: i128,

    /// Change in fixed-vector lines.
    pub vector_lines: i128,

    /// Changes in classified `call`, `invoke`, and `callbr` instructions.
    pub call_sites: CallSiteDelta,

    /// Change in safety-check references.
    pub safety_checks: i128,
}

impl BodySetDelta {
    pub(crate) fn between(before: &BodySetSummary, after: &BodySetSummary) -> Self {
        Self {
            bodies: delta(before.bodies, after.bodies),
            bytes: delta(before.bytes, after.bytes),
            instructions: delta(before.instructions, after.instructions),
            vector_lines: delta(before.vector_lines, after.vector_lines),
            call_sites: CallSiteDelta::between(&before.call_sites, &after.call_sites),
            safety_checks: delta(before.safety_checks, after.safety_checks),
        }
    }
}

fn delta(before: usize, after: usize) -> i128 {
    after as i128 - before as i128
}

/// A structural comparison of two exact instances at one compiler stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompareView {
    /// The selected compiler output.
    pub output: CompilerOutput,

    /// The first instance.
    pub before_instance: InstanceSummary,

    /// The second instance.
    pub after_instance: InstanceSummary,

    /// Capture dimensions that differ and can invalidate a direct comparison.
    pub compatibility_differences: Vec<String>,

    /// Aggregated structure before the change.
    pub before: BodySetSummary,

    /// Aggregated structure after the change.
    pub after: BodySetSummary,

    /// Signed structural changes from before to after.
    pub delta: BodySetDelta,
}

#[cfg(test)]
mod tests {
    use super::{BodySetDelta, BodySetSummary, BodyView, LlvmBodySummary};
    use crate::LlvmStage;

    fn body(text: &str) -> BodyView {
        BodyView {
            stage: LlvmStage::Optimized,
            module: "module".to_owned(),
            symbol: "symbol".to_owned(),
            text: text.to_owned(),
            summary: LlvmBodySummary::from_text(text),
        }
    }

    #[test]
    fn summarizes_vector_calls_and_safety_checks() {
        let summary = LlvmBodySummary::from_text(concat!(
            "define void @kernel(ptr %callback) {\n",
            "  %values = load <4 x i32>, ptr null\n",
            "  %result = call i32 %callback(i32 1)\n",
            "  call void @panic_bounds_check()\n",
            "  ret void\n",
            "}\n",
        ));

        assert_eq!(summary.instructions, 4);
        assert_eq!(summary.vector_lines, 1);
        assert_eq!(summary.vector_widths, [4]);
        assert_eq!(summary.call_sites.total, 2);
        assert_eq!(summary.call_sites.direct_non_intrinsic, 1);
        assert_eq!(summary.call_sites.indirect, 1);
        assert_eq!(summary.safety_checks, 1);
    }

    #[test]
    fn aggregates_and_compares_call_site_categories() {
        let runtime_and_memory = body(concat!(
            "define void @first() {\n",
            "  call void @runtime()\n",
            "  call void @llvm.memset.p0.i64(ptr null, i8 0, i64 8, i1 false)\n",
            "  ret void\n",
            "}\n",
        ));
        let indirect_and_assembly = body(concat!(
            "define void @second(ptr %callback) {\n",
            "  invoke void %callback() to label %ok unwind label %error\n",
            "ok:\n",
            "  callbr void asm sideeffect \"\", \"\"() to label %exit [label %error]\n",
            "exit:\n",
            "  ret void\n",
            "error:\n",
            "  unreachable\n",
            "}\n",
        ));

        let before = BodySetSummary::from_bodies(std::slice::from_ref(&runtime_and_memory));
        let after = BodySetSummary::from_bodies(&[runtime_and_memory, indirect_and_assembly]);
        let delta = BodySetDelta::between(&before, &after);

        assert_eq!(after.call_sites.total, 4);
        assert_eq!(after.call_sites.direct_non_intrinsic, 1);
        assert_eq!(after.call_sites.indirect, 1);
        assert_eq!(after.call_sites.inline_asm, 1);
        assert_eq!(after.call_sites.memory_intrinsics, 1);
        assert_eq!(delta.call_sites.total, 2);
        assert_eq!(delta.call_sites.indirect, 1);
        assert_eq!(delta.call_sites.inline_asm, 1);
    }
}
