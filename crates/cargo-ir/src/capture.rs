//! Runs one Cargo analysis and collects supported LLVM modules.
//!
//! [`compile_with_events`] uses `cargo rustc` so normal and analysis builds share dependency
//! artifacts. [`compile`] discards its user-visible Cargo events. The selected target has a
//! separate Cargo identity because saved compiler temporaries are part of Cargo's fingerprint.
//! [`ingest_with_events`] reads the retained artifacts without invoking Cargo again.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{ErrorKind, Read};
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::cargo_output::{self, CargoProcessEvent};
use crate::driver::RustcDriver;
use crate::llvm;
use crate::mono;
#[cfg(test)]
use crate::parse_optimization_remarks;
use crate::toolchain::{CargoContext, inspect_rustc};
use crate::{
    BuildRequest, CaptureProfile, CargoTarget, CompilerPlacement, Error, OptimizationRemark,
    RemarkParseLimits, Result, Toolchain, parse_optimization_remarks_with,
};

const REMARKS_DIRECTORY_NAME: &str = "remarks";
const MAX_REMARK_FILES: usize = 4_096;
const MAX_REMARK_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_REMARK_RECORDS: usize = 5_000_000;
const MAX_DISASSEMBLER_DIAGNOSTIC_BYTES: usize = 1024 * 1024;

/// The byte range of one LLVM function definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BodyRange {
    /// The raw LLVM symbol without its leading `@`.
    pub raw_symbol: String,

    /// The best available demangled display name.
    pub demangled: String,

    /// The inclusive byte offset at which the definition starts.
    pub start: u64,

    /// The exclusive byte offset at which the definition ends.
    pub end: u64,
}

/// The byte range of one LLVM function declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmDeclaration {
    /// The raw LLVM symbol without its leading `@`.
    pub raw_symbol: String,

    /// The best available demangled display name.
    pub demangled: String,

    /// The inclusive byte offset at which the declaration starts.
    pub start: u64,

    /// The exclusive byte offset at which the declaration ends.
    pub end: u64,
}

/// The exact relationship represented by an LLVM alias.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum AliasTarget {
    /// The alias directly names one target symbol.
    Symbol {
        /// The target's raw LLVM symbol without its leading `@`.
        raw_symbol: String,
    },

    /// The aliasee is a constant expression or cannot be represented as one symbol.
    Expression,
}

/// The byte range and direct relationship of one LLVM alias.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlvmAlias {
    /// The raw LLVM symbol without its leading `@`.
    pub raw_symbol: String,

    /// The best available demangled display name.
    pub demangled: String,

    /// The exact direct target, or an explicit expression marker.
    pub target: AliasTarget,

    /// The inclusive byte offset at which the alias starts.
    pub start: u64,

    /// The exclusive byte offset at which the alias ends.
    pub end: u64,
}

/// How cargo-ir obtained an artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureMethod {
    /// rustc wrote the artifact because cargo-ir enabled saved temporaries.
    SavedTemporary,
}

/// The link-time optimization scope implied by an artifact stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LtoScope {
    /// The artifact is outside an LTO pipeline.
    None,

    /// The artifact belongs to ThinLTO.
    Thin,

    /// The filename does not establish an LTO scope.
    Unknown,
}

/// Exact compiler provenance for one LLVM artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactProvenance {
    /// The normalized stage, when cargo-ir understands the compiler stage.
    pub stage: Option<LlvmStage>,

    /// The exact saved-temporary stage suffix emitted by rustc.
    pub compiler_stage: String,

    /// The codegen unit inferred from the compiler-owned filename, when present.
    pub codegen_unit: Option<String>,

    /// The LTO scope implied by the exact compiler stage.
    pub lto: LtoScope,

    /// The mechanism that produced the artifact.
    pub capture_method: CaptureMethod,
}

/// One raw LLVM optimization-remark file and its parsed records.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemarkEvidence {
    /// The normalized compiler-owned path relative to the remark directory.
    pub name: String,

    /// The compiler-owned YAML file.
    pub raw_path: PathBuf,

    /// The typed records parsed from the raw YAML document stream.
    pub records: Vec<OptimizationRemark>,
}

/// Capture metadata that is available before evidence records are streamed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceMetadata {
    /// The request and effective compiler invocation.
    pub invocation: CaptureInvocation,

    /// The exact analyzed compiler.
    pub toolchain: Toolchain,

    /// Whether the capture requested optimization remarks.
    pub remarks_captured: bool,
}

/// Metadata for one module before its symbol records are streamed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleStart {
    /// The compiler-owned artifact file name.
    pub name: String,

    /// The compiler stage and collection method for this artifact.
    pub provenance: ArtifactProvenance,

    /// The saved LLVM bitcode path.
    pub bitcode_path: PathBuf,

    /// The matching textual LLVM module path.
    pub text_path: PathBuf,
}

/// Metadata for one optimization-remark file before its records are streamed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemarkFileStart {
    /// The normalized compiler-owned path relative to the remark directory.
    pub name: String,

    /// The compiler-owned YAML file.
    pub raw_path: PathBuf,
}

/// One bounded record in a retained compiler-evidence stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum EvidenceEvent {
    /// One rustc monomorphization placement.
    Placement {
        /// The placement record.
        record: CompilerPlacement,
    },

    /// The start of one disassembled LLVM module.
    ModuleStarted {
        /// The module metadata.
        module: ModuleStart,
    },

    /// One function definition range in the current module.
    Body {
        /// The indexed function body.
        body: BodyRange,
    },

    /// One function declaration range in the current module.
    Declaration {
        /// The indexed declaration.
        declaration: LlvmDeclaration,
    },

    /// One LLVM alias range in the current module.
    Alias {
        /// The indexed alias.
        alias: LlvmAlias,
    },

    /// The start of one raw optimization-remark file.
    RemarkFileStarted {
        /// The remark-file metadata.
        file: RemarkFileStart,
    },

    /// One optimization-remark document in the current file.
    Remark {
        /// The parsed remark.
        remark: OptimizationRemark,
    },
}

/// A supported stage of the LLVM compilation pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LlvmStage {
    /// LLVM IR before the LLVM optimization pipeline.
    #[serde(rename = "llvm-pre-optimization")]
    PreOptimization,

    /// LLVM IR after the LLVM optimization pipeline.
    #[serde(rename = "llvm-optimized")]
    Optimized,
}

impl LlvmStage {
    /// Returns the stable catalog name for this stage.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreOptimization => "llvm-pre-optimization",
            Self::Optimized => "llvm-optimized",
        }
    }
}

/// One subprocess command involved in a capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandInvocation {
    /// The executable passed to the operating system.
    pub program: String,

    /// The exact Unicode arguments supplied to the executable.
    pub arguments: Vec<String>,
}

/// One non-secret compiler environment variable that can affect code generation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    /// The variable name.
    pub name: String,

    /// The variable value.
    pub value: String,
}

/// One unstable-access mechanism that Optic can use in a bounded compiler subprocess.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnstableAccessMechanism {
    /// Optic set `RUSTC_BOOTSTRAP=1` for a bounded child process.
    RustcBootstrap,
}

/// One child-process scope in which Optic is authorized to enable unstable compiler access.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnstableAccessScope {
    /// Reading Cargo's merged configuration.
    CargoConfigDiscovery,

    /// Building the exact-version rustc-private driver.
    DriverBuild,

    /// Running rustc for the selected Cargo target.
    SelectedTarget,
}

/// The bounded unstable-access policy for one capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnstableAccess {
    /// The mechanism available to each authorized scope.
    pub mechanism: UnstableAccessMechanism,

    /// The only child-process scopes in which Optic can use the mechanism.
    ///
    /// A scope can be authorized without running. For example, Optic does not build a cached
    /// compiler driver again.
    #[serde(alias = "scopes")]
    pub authorized_scopes: Vec<UnstableAccessScope>,
}

/// The request and effective process metadata for one Cargo analysis.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CaptureInvocation {
    /// The normalized request accepted by cargo-ir.
    pub request: BuildRequest,

    /// The exact Cargo command constructed by cargo-ir.
    pub cargo: CommandInvocation,

    /// The selected rustc command, when Cargo invoked rustc.
    pub rustc: Option<CommandInvocation>,

    /// The effective outer-to-inner wrapper chain known to cargo-ir.
    pub wrapper_chain: Vec<String>,

    /// Codegen-related environment variables inherited by Cargo.
    pub environment: Vec<EnvironmentVariable>,

    /// Compiler arguments injected to collect evidence or implement the capture profile.
    pub injected_rustc_arguments: Vec<String>,

    /// The bounded policy for unstable compiler access.
    pub unstable_access: UnstableAccess,
}

/// The result of asking Cargo for evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum CompileOutcome {
    /// Cargo invoked rustc and cargo-ir collected new evidence.
    Compiled {
        /// Metadata needed to ingest the retained compiler artifacts.
        compilation: Box<CompiledCapture>,
    },

    /// Cargo completed without invoking the selected rustc driver and reported fresh artifacts.
    Fresh {
        /// Request and Cargo invocation metadata. `rustc` is absent.
        invocation: Box<CaptureInvocation>,

        /// The number of fresh compiler-artifact events that Cargo reported.
        artifact_count: usize,
    },
}

/// Metadata retained between compiler execution and evidence ingestion.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompiledCapture {
    /// The request and effective compiler invocation.
    pub invocation: CaptureInvocation,

    /// The exact analyzed compiler.
    pub toolchain: Toolchain,
}

/// Runs one analysis for the selected Cargo target and discards its Cargo output events.
///
/// # Errors
///
/// Returns an error from [`compile_with_events`].
pub fn compile(request: &BuildRequest) -> Result<CompileOutcome> {
    compile_with_events(request, |_| ControlFlow::Continue(()))
}

/// Runs one analysis and reports user-visible Cargo output as it arrives.
///
/// # Errors
///
/// Returns an error if the toolchain is unsupported, the analysis directory is not empty, Cargo
/// fails, Cargo exceeds an output bound, or emitted LLVM evidence cannot be read.
pub fn compile_with_events(
    request: &BuildRequest,
    mut on_event: impl FnMut(CargoProcessEvent) -> ControlFlow<()>,
) -> Result<CompileOutcome> {
    let cargo = CargoContext::discover(&request.workspace_root)?;
    let toolchain = inspect_rustc(&cargo)?;
    prepare_analysis_directory(&request.analysis_directory)?;
    if request.capture_remarks {
        prepare_remarks_directory(&request.analysis_directory)?;
    }
    let driver = RustcDriver::prepare(&toolchain, &cargo)?;
    let manifest_path = request.analysis_directory.join(mono::MANIFEST_NAME);

    let mut command = cargo_command(request, &cargo);
    driver.configure(
        &mut command,
        &request.analysis_directory,
        &manifest_path,
        &toolchain.commit_hash,
    );
    let invocation = CaptureInvocation {
        request: request.clone(),
        cargo: command_invocation(&command),
        rustc: None,
        wrapper_chain: driver.wrapper_chain(),
        environment: compiler_environment(),
        injected_rustc_arguments: injected_rustc_arguments(request),
        unstable_access: UnstableAccess {
            mechanism: UnstableAccessMechanism::RustcBootstrap,
            authorized_scopes: vec![
                UnstableAccessScope::CargoConfigDiscovery,
                UnstableAccessScope::DriverBuild,
                UnstableAccessScope::SelectedTarget,
            ],
        },
    };
    let output = cargo_output::run(&mut command, "cargo rustc", &mut on_event)?;
    if !output.status().success() {
        return Err(Error::ProcessFailed {
            program: "cargo rustc".to_owned(),
            status: output.status().to_string(),
            diagnostics: output.diagnostics(),
        });
    }

    if !manifest_path.is_file() {
        if let Some(artifact_count) = output.fresh_artifact_count() {
            return Ok(CompileOutcome::Fresh {
                invocation: Box::new(invocation),
                artifact_count,
            });
        }

        return Err(Error::MissingEvidence);
    }

    require_compiled_evidence(request)?;

    Ok(CompileOutcome::Compiled {
        compilation: Box::new(CompiledCapture {
            invocation,
            toolchain,
        }),
    })
}

/// Asks Cargo whether retained evidence still represents the selected target.
///
/// This check never compiles the selected target. The Optic wrapper stops a stale target before
/// rustc starts, so retained compiler artifacts remain unchanged.
///
/// # Errors
///
/// Returns an error if Cargo or compiler discovery fails for a reason other than staleness.
pub fn check_fresh(request: &BuildRequest, expected_toolchain: &Toolchain) -> Result<bool> {
    const FRESHNESS_CHECK_ENV: &str = "OPTIC_FRESHNESS_CHECK";
    const FRESHNESS_STALE_DIAGNOSTIC: &str = "cargo-optic selected target is not fresh";

    let cargo = CargoContext::discover(&request.workspace_root)?;
    let toolchain = inspect_rustc(&cargo)?;
    if &toolchain != expected_toolchain {
        return Ok(false);
    }
    let driver = RustcDriver::prepare(&toolchain, &cargo)?;
    let manifest_path = request.analysis_directory.join(mono::MANIFEST_NAME);
    let mut command = cargo_command(request, &cargo);
    driver.configure(
        &mut command,
        &request.analysis_directory,
        &manifest_path,
        &toolchain.commit_hash,
    );
    command.env(FRESHNESS_CHECK_ENV, "1");
    let output = cargo_output::run(&mut command, "cargo rustc freshness check", &mut |_| {
        ControlFlow::Continue(())
    })?;
    if !output.status().success() {
        let diagnostics = output.diagnostics();
        if diagnostics.contains(FRESHNESS_STALE_DIAGNOSTIC) {
            return Ok(false);
        }

        return Err(Error::ProcessFailed {
            program: "cargo rustc freshness check".to_owned(),
            status: output.status().to_string(),
            diagnostics,
        });
    }

    Ok(output.fresh_artifact_count().is_some())
}

/// Confirms that a successful compiler run left the identity manifest and supported bitcode.
///
/// # Errors
///
/// Returns an error if either required artifact class is absent or cannot be inspected.
pub fn require_compiled_evidence(request: &BuildRequest) -> Result<()> {
    let manifest_path = request.analysis_directory.join(mono::MANIFEST_NAME);
    if !manifest_path.is_file() || supported_bitcode(&request.analysis_directory)?.is_empty() {
        return Err(Error::MissingEvidence);
    }

    Ok(())
}

/// Streams retained compiler evidence without invoking Cargo again.
///
/// The callback runs on the calling thread. Each event owns at most one bounded evidence record.
///
/// # Errors
///
/// Returns an error if the manifest, LLVM bitcode, textual LLVM output, or remarks are invalid.
pub fn ingest_with_events(
    request: &BuildRequest,
    mut compilation: CompiledCapture,
    mut on_event: impl FnMut(EvidenceEvent),
) -> Result<EvidenceMetadata> {
    require_compiled_evidence(request)?;
    compilation.invocation.request = request.clone();
    let manifest_path = request.analysis_directory.join(mono::MANIFEST_NAME);
    let mut manifest =
        mono::CompilerManifestReader::open(&manifest_path, &compilation.toolchain.commit_hash)?;
    compilation.invocation.rustc = Some(compiler_invocation(manifest.rustc_arguments())?);

    while let Some(record) = manifest.next_placement()? {
        on_event(EvidenceEvent::Placement { record });
    }

    for artifact in supported_bitcode(&request.analysis_directory)? {
        disassemble_with_events(
            &compilation.toolchain,
            artifact,
            &request.analysis_directory,
            &mut on_event,
        )?;
    }

    if request.capture_remarks {
        stream_remarks(
            &remarks_directory(&request.analysis_directory),
            RemarkCollectionLimits::default(),
            &mut on_event,
        )?;
    }

    Ok(EvidenceMetadata {
        invocation: compilation.invocation,
        toolchain: compilation.toolchain,
        remarks_captured: request.capture_remarks,
    })
}

fn cargo_command(request: &BuildRequest, cargo: &CargoContext) -> Command {
    let mut command = Command::new(cargo.cargo());
    command.current_dir(&request.workspace_root);
    command.arg("rustc");
    command.arg("--message-format=json-render-diagnostics");

    if let Some(path) = &request.manifest_path {
        command.arg("--manifest-path").arg(path);
    }
    if let Some(package) = &request.package {
        command.arg("--package").arg(package);
    }
    if let Some(target) = &request.target {
        match target {
            CargoTarget::Library => {
                command.arg("--lib");
            }
            CargoTarget::Binary(name) => {
                command.arg("--bin").arg(name);
            }
            CargoTarget::Benchmark(name) => {
                command.arg("--bench").arg(name);
            }
            CargoTarget::Example(name) => {
                command.arg("--example").arg(name);
            }
        }
    }
    if let Some(profile) = &request.profile {
        command.arg("--profile").arg(profile);
    }
    if !request.features.is_empty() {
        command.arg("--features").arg(request.features.join(","));
    }
    if request.all_features {
        command.arg("--all-features");
    }
    if request.no_default_features {
        command.arg("--no-default-features");
    }
    if let Some(target) = &request.target_triple {
        command.arg("--target").arg(target);
    }
    if request.locked {
        command.arg("--locked");
    }
    if request.offline {
        command.arg("--offline");
    }
    if request.frozen {
        command.arg("--frozen");
    }

    command.arg("--");
    command.args(injected_rustc_arguments(request));

    command
}

fn injected_rustc_arguments(request: &BuildRequest) -> Vec<String> {
    let mut arguments = vec![
        "-C".to_owned(),
        "save-temps".to_owned(),
        "-Z".to_owned(),
        temps_argument(&request.analysis_directory)
            .to_string_lossy()
            .into_owned(),
    ];

    if request.capture_remarks {
        arguments.extend([
            "-C".to_owned(),
            "remark=all".to_owned(),
            "-Z".to_owned(),
            remarks_argument(&remarks_directory(&request.analysis_directory))
                .to_string_lossy()
                .into_owned(),
        ]);
    }

    match &request.capture_profile {
        CaptureProfile::Faithful => {}
        CaptureProfile::Enriched => {
            arguments.extend([
                "-C".to_owned(),
                "symbol-mangling-version=v0".to_owned(),
                "-C".to_owned(),
                "debuginfo=line-tables-only".to_owned(),
            ]);
        }
        CaptureProfile::Experiment { rustc_arguments } => {
            arguments.extend(rustc_arguments.iter().cloned());
        }
    }

    arguments
}

fn temps_argument(path: &Path) -> OsString {
    let mut argument = OsString::from("temps-dir=");
    argument.push(path);
    argument
}

fn remarks_argument(path: &Path) -> OsString {
    let mut argument = OsString::from("remark-dir=");
    argument.push(path);

    argument
}

fn command_invocation(command: &Command) -> CommandInvocation {
    CommandInvocation {
        program: command.get_program().to_string_lossy().into_owned(),
        arguments: command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect(),
    }
}

fn compiler_invocation(arguments: &[String]) -> Result<CommandInvocation> {
    let Some((program, arguments)) = arguments.split_first() else {
        return Err(Error::InvalidIdentityManifest {
            path: PathBuf::from(mono::MANIFEST_NAME),
            message: "rustc invocation does not contain a compiler path".to_owned(),
        });
    };

    Ok(CommandInvocation {
        program: program.clone(),
        arguments: arguments.to_vec(),
    })
}

fn compiler_environment() -> Vec<EnvironmentVariable> {
    let mut variables = env::vars()
        .filter(|(name, _)| is_compiler_environment(name))
        .map(|(name, value)| EnvironmentVariable { name, value })
        .collect::<Vec<_>>();
    variables.sort_by(|left, right| left.name.cmp(&right.name));

    variables
}

fn is_compiler_environment(name: &str) -> bool {
    matches!(
        name,
        "CARGO_BUILD_RUSTC"
            | "CARGO_BUILD_RUSTC_WRAPPER"
            | "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"
            | "CARGO_ENCODED_RUSTFLAGS"
            | "CARGO_TARGET_DIR"
            | "RUSTC"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "RUSTFLAGS"
    ) || name.starts_with("CARGO_PROFILE_")
        || name.starts_with("CARGO_TARGET_")
}

fn prepare_analysis_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::Filesystem {
        operation: "create",
        path: path.to_owned(),
        source,
    })?;
    let mut entries = fs::read_dir(path).map_err(|source| Error::Filesystem {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;

    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) => Err(Error::AnalysisDirectoryNotEmpty {
            path: path.to_owned(),
        }),
        Some(Err(source)) => Err(Error::Filesystem {
            operation: "read",
            path: path.to_owned(),
            source,
        }),
    }
}

fn prepare_remarks_directory(analysis_directory: &Path) -> Result<()> {
    let path = remarks_directory(analysis_directory);

    fs::create_dir(&path).map_err(|source| Error::Filesystem {
        operation: "create",
        path,
        source,
    })
}

fn remarks_directory(analysis_directory: &Path) -> PathBuf {
    analysis_directory.join(REMARKS_DIRECTORY_NAME)
}

struct BitcodeArtifact {
    path: PathBuf,
    provenance: ArtifactProvenance,
}

#[derive(Clone, Copy)]
struct RemarkCollectionLimits {
    max_files: usize,
    max_bytes: u64,
    max_records: usize,
    parse: RemarkParseLimits,
}

impl Default for RemarkCollectionLimits {
    fn default() -> Self {
        Self {
            max_files: MAX_REMARK_FILES,
            max_bytes: MAX_REMARK_BYTES,
            max_records: MAX_REMARK_RECORDS,
            parse: RemarkParseLimits::default(),
        }
    }
}

#[cfg(test)]
fn collect_remarks(directory: &Path) -> Result<Vec<RemarkEvidence>> {
    collect_remarks_with_limits(directory, RemarkCollectionLimits::default())
}

#[cfg(test)]
fn requested_remarks(request: &BuildRequest) -> Result<Option<Vec<RemarkEvidence>>> {
    request
        .capture_remarks
        .then(|| collect_remarks(&remarks_directory(&request.analysis_directory)))
        .transpose()
}

#[cfg(test)]
fn collect_remarks_with_limits(
    directory: &Path,
    limits: RemarkCollectionLimits,
) -> Result<Vec<RemarkEvidence>> {
    let paths = remark_paths(directory, limits)?;
    let mut evidence = Vec::with_capacity(paths.len());
    let mut total_records = 0_usize;

    for raw_path in paths {
        let name = normalized_remark_name(directory, &raw_path)?;
        let records = parse_optimization_remarks(&raw_path, limits.parse)?;
        total_records = total_records.saturating_add(records.len());
        validate_remark_record_count(directory, limits, total_records)?;
        evidence.push(RemarkEvidence {
            name,
            raw_path,
            records,
        });
    }

    Ok(evidence)
}

fn stream_remarks(
    directory: &Path,
    limits: RemarkCollectionLimits,
    on_event: &mut impl FnMut(EvidenceEvent),
) -> Result<()> {
    let paths = remark_paths(directory, limits)?;
    let mut total_records = 0_usize;

    for raw_path in paths {
        let name = normalized_remark_name(directory, &raw_path)?;
        on_event(EvidenceEvent::RemarkFileStarted {
            file: RemarkFileStart {
                name,
                raw_path: raw_path.clone(),
            },
        });
        parse_optimization_remarks_with(&raw_path, limits.parse, |remark| {
            total_records = total_records.saturating_add(1);
            on_event(EvidenceEvent::Remark { remark });
        })?;
        validate_remark_record_count(directory, limits, total_records)?;
    }

    Ok(())
}

fn remark_paths(directory: &Path, limits: RemarkCollectionLimits) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut total_bytes = 0_u64;

    for entry in WalkDir::new(directory).min_depth(1) {
        let entry = entry.map_err(|source| Error::Filesystem {
            operation: "walk",
            path: directory.to_owned(),
            source: source.into(),
        })?;
        if !entry.file_type().is_file() || !is_optimization_remark(entry.file_name()) {
            continue;
        }

        if paths.len() >= limits.max_files {
            return Err(invalid_remark_collection(
                directory,
                format!(
                    "file count exceeds {}, got {}",
                    limits.max_files,
                    paths.len() + 1
                ),
            ));
        }

        let path = entry.into_path();
        let length = fs::metadata(&path)
            .map_err(|source| Error::Filesystem {
                operation: "read metadata for",
                path: path.clone(),
                source,
            })?
            .len();
        total_bytes = total_bytes.saturating_add(length);
        if total_bytes > limits.max_bytes {
            return Err(invalid_remark_collection(
                directory,
                format!(
                    "aggregate file length exceeds {} bytes, got {total_bytes}",
                    limits.max_bytes
                ),
            ));
        }

        paths.push(path);
    }

    paths.sort();

    Ok(paths)
}

fn validate_remark_record_count(
    directory: &Path,
    limits: RemarkCollectionLimits,
    total_records: usize,
) -> Result<()> {
    if total_records > limits.max_records {
        return Err(invalid_remark_collection(
            directory,
            format!(
                "aggregate record count exceeds {}, got {total_records}",
                limits.max_records
            ),
        ));
    }

    Ok(())
}

fn normalized_remark_name(directory: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(directory).map_err(|_| {
        invalid_remark_collection(
            path,
            "remark path must be below the remark directory".to_owned(),
        )
    })?;
    let mut components = Vec::new();

    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(invalid_remark_collection(
                path,
                "remark path must contain only normal relative components".to_owned(),
            ));
        };
        let component = component.to_str().ok_or_else(|| {
            invalid_remark_collection(path, "remark path must be valid UTF-8".to_owned())
        })?;
        components.push(component);
    }
    if components.is_empty() {
        return Err(invalid_remark_collection(
            path,
            "remark path must contain a file name".to_owned(),
        ));
    }

    Ok(components.join("/"))
}

fn is_optimization_remark(name: &OsStr) -> bool {
    name.to_string_lossy().ends_with(".opt.opt.yaml")
}

fn invalid_remark_collection(path: &Path, message: String) -> Error {
    Error::InvalidOptimizationRemarks {
        path: path.to_owned(),
        message,
    }
}

fn supported_bitcode(directory: &Path) -> Result<Vec<BitcodeArtifact>> {
    let mut artifacts = Vec::new();

    for entry in WalkDir::new(directory).min_depth(1) {
        let entry = entry.map_err(|source| Error::Filesystem {
            operation: "walk",
            path: directory.to_owned(),
            source: source.into(),
        })?;

        if !entry.file_type().is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".bc") {
            artifacts.push(BitcodeArtifact {
                path: entry.into_path(),
                provenance: artifact_provenance(&name),
            });
        }
    }

    artifacts.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(artifacts)
}

fn artifact_provenance(name: &str) -> ArtifactProvenance {
    let (compiler_stage, stage, lto) = if name.ends_with(".no-opt.bc") {
        (
            "no-opt".to_owned(),
            Some(LlvmStage::PreOptimization),
            LtoScope::None,
        )
    } else if name.ends_with(".thin-lto-input.bc") {
        ("thin-lto-input".to_owned(), None, LtoScope::Thin)
    } else if name.ends_with(".thin-lto-after-import.bc") {
        ("thin-lto-after-import".to_owned(), None, LtoScope::Thin)
    } else if name.ends_with(".thin-lto-after-internalize.bc") {
        (
            "thin-lto-after-internalize".to_owned(),
            None,
            LtoScope::Thin,
        )
    } else if name.ends_with(".thin-lto-after-pm.bc") {
        (
            "thin-lto-after-pm".to_owned(),
            Some(LlvmStage::Optimized),
            LtoScope::Thin,
        )
    } else if name.contains(".thin-lto-") {
        (unknown_compiler_stage(name), None, LtoScope::Thin)
    } else if name.ends_with(".rcgu.bc") {
        (
            "rcgu".to_owned(),
            Some(LlvmStage::Optimized),
            LtoScope::None,
        )
    } else {
        (unknown_compiler_stage(name), None, LtoScope::Unknown)
    };
    let codegen_unit = codegen_unit(name, &compiler_stage);

    ArtifactProvenance {
        stage,
        compiler_stage,
        codegen_unit,
        lto,
        capture_method: CaptureMethod::SavedTemporary,
    }
}

fn unknown_compiler_stage(name: &str) -> String {
    let stem = name.strip_suffix(".bc").unwrap_or(name);

    stem.rsplit_once(".rcgu.")
        .map_or(stem, |(_, compiler_stage)| compiler_stage)
        .to_owned()
}

fn codegen_unit(name: &str, compiler_stage: &str) -> Option<String> {
    let suffix = format!(".{compiler_stage}.bc");
    let prefix = name.strip_suffix(&suffix)?;
    let codegen_unit = prefix.strip_suffix(".rcgu").unwrap_or(prefix);

    Some(codegen_unit.to_owned())
}

fn disassemble_with_events(
    toolchain: &Toolchain,
    artifact: BitcodeArtifact,
    analysis_directory: &Path,
    on_event: &mut impl FnMut(EvidenceEvent),
) -> Result<()> {
    let module = disassemble_module(toolchain, artifact, analysis_directory)?;
    let text_path = module.text_path.clone();
    on_event(EvidenceEvent::ModuleStarted { module });
    let scan = llvm::scan_with(&text_path, |record| {
        on_event(match record {
            llvm::ModuleRecord::Body(body) => EvidenceEvent::Body { body },
            llvm::ModuleRecord::Declaration(declaration) => {
                EvidenceEvent::Declaration { declaration }
            }
            llvm::ModuleRecord::Alias(alias) => EvidenceEvent::Alias { alias },
        });
    });

    remove_generated_llvm_text(&text_path, scan)
}

fn disassemble_module(
    toolchain: &Toolchain,
    artifact: BitcodeArtifact,
    analysis_directory: &Path,
) -> Result<ModuleStart> {
    let BitcodeArtifact {
        path: bitcode_path,
        provenance,
    } = artifact;
    let file_name = bitcode_path
        .file_name()
        .expect("bitcode artifacts come from file entries in the analysis directory");
    let name = file_name.to_string_lossy().into_owned();
    let mut text_name = file_name.to_owned();
    text_name.push(".ll");
    let text_path = analysis_directory.join(text_name);
    let (status, diagnostics) = match run_disassembler(toolchain, &bitcode_path, &text_path) {
        Ok(output) => output,
        Err(error) => return remove_generated_llvm_text(&text_path, Err(error)),
    };
    if !status.success() {
        let error = Error::ProcessFailed {
            program: toolchain.llvm_dis.display().to_string(),
            status: status.to_string(),
            diagnostics: String::from_utf8_lossy(&diagnostics).into_owned(),
        };

        return remove_generated_llvm_text(&text_path, Err(error));
    }

    Ok(ModuleStart {
        name,
        provenance,
        bitcode_path,
        text_path,
    })
}

fn remove_generated_llvm_text<T>(text_path: &Path, result: Result<T>) -> Result<T> {
    let removal = match fs::remove_file(text_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Filesystem {
            operation: "remove",
            path: text_path.to_owned(),
            source,
        }),
    };

    match result {
        Err(error) => Err(error),
        Ok(value) => {
            removal?;

            Ok(value)
        }
    }
}

fn run_disassembler(
    toolchain: &Toolchain,
    bitcode_path: &Path,
    text_path: &Path,
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    let mut child = Command::new(&toolchain.llvm_dis)
        .arg("-o")
        .arg(text_path)
        .arg(bitcode_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| Error::StartProcess {
            program: toolchain.llvm_dis.display().to_string(),
            source,
        })?;
    let mut stderr = child
        .stderr
        .take()
        .expect("llvm-dis stderr was configured as a pipe before the child started");
    let mut diagnostics = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];

    loop {
        let bytes = stderr
            .read(&mut buffer)
            .map_err(|source| Error::Filesystem {
                operation: "read diagnostics from",
                path: toolchain.llvm_dis.clone(),
                source,
            })?;
        if bytes == 0 {
            break;
        }

        diagnostics.extend_from_slice(&buffer[..bytes]);
        let excess = diagnostics
            .len()
            .saturating_sub(MAX_DISASSEMBLER_DIAGNOSTIC_BYTES);
        if excess != 0 {
            diagnostics.drain(..excess);
        }
    }
    let status = child.wait().map_err(|source| Error::Filesystem {
        operation: "wait for",
        path: toolchain.llvm_dis.clone(),
        source,
    })?;

    Ok((status, diagnostics))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;

    use super::{
        RemarkCollectionLimits, artifact_provenance, collect_remarks_with_limits,
        injected_rustc_arguments, prepare_analysis_directory, prepare_remarks_directory,
        remove_generated_llvm_text, requested_remarks,
    };
    use crate::{
        BuildRequest, CaptureProfile, Error, LlvmStage, LtoScope, UnstableAccess,
        UnstableAccessScope,
    };

    const REMARK: &str = r#"--- !Passed
Pass: inline
Name: Inlined
Function: _Z8functionv
Args:
  - String: inlined
...
"#;

    #[test]
    fn unstable_access_serializes_authorized_scopes_and_accepts_the_legacy_name() {
        let policy = serde_json::from_value::<UnstableAccess>(serde_json::json!({
            "mechanism": "rustc-bootstrap",
            "scopes": ["selected-target"],
        }))
        .expect("the legacy scope name remains readable");

        assert_eq!(
            policy.authorized_scopes,
            vec![UnstableAccessScope::SelectedTarget]
        );
        assert_eq!(
            serde_json::to_value(policy).expect("the policy is serializable"),
            serde_json::json!({
                "mechanism": "rustc-bootstrap",
                "authorized_scopes": ["selected-target"],
            })
        );
    }

    #[test]
    fn rejects_a_nonempty_analysis_directory() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        fs::write(temporary.path().join("stale.bc"), b"stale")
            .expect("the test can create stale evidence");

        assert!(matches!(
            prepare_analysis_directory(temporary.path()),
            Err(Error::AnalysisDirectoryNotEmpty { path }) if path == temporary.path()
        ));
    }

    #[test]
    fn removes_generated_llvm_text_after_a_successful_scan() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let text = temporary.path().join("module.bc.ll");
        fs::write(&text, b"derived LLVM text").expect("the test can create generated LLVM text");

        let value = remove_generated_llvm_text(&text, Ok("indexed"))
            .expect("the generated LLVM text can be removed");

        assert_eq!(value, "indexed");
        assert!(!text.exists());
    }

    #[test]
    fn removes_generated_llvm_text_after_a_failed_scan() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let text = temporary.path().join("module.bc.ll");
        fs::write(&text, b"invalid LLVM text").expect("the test can create generated LLVM text");
        let scan = Err(Error::InvalidLlvm {
            path: text.clone(),
            message: "test scan failure".to_owned(),
        });

        let result = remove_generated_llvm_text::<()>(&text, scan);

        assert!(matches!(result, Err(Error::InvalidLlvm { .. })));
        assert!(!text.exists());
    }

    #[test]
    fn preserves_a_scan_error_when_generated_text_removal_fails() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let text = temporary.path().join("module.bc.ll");
        fs::create_dir(&text).expect("the test can create an invalid generated-text directory");
        let scan = Err(Error::InvalidLlvm {
            path: text.clone(),
            message: "test scan failure".to_owned(),
        });

        let result = remove_generated_llvm_text::<()>(&text, scan);

        assert!(matches!(
            result,
            Err(Error::InvalidLlvm { message, .. }) if message == "test scan failure"
        ));
    }

    #[test]
    fn faithful_capture_changes_only_evidence_emission() {
        let arguments = injected_rustc_arguments(&request(CaptureProfile::Faithful));

        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-C", "save-temps"])
        );
        assert!(!arguments.iter().any(|argument| argument == "no-link"));
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("symbol-mangling-version="))
        );
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("debuginfo="))
        );
        assert!(!arguments.iter().any(|argument| argument == "remark=all"));
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("remark-dir="))
        );
    }

    #[test]
    fn enriched_and_experiment_profiles_record_their_arguments() {
        let enriched = injected_rustc_arguments(&request(CaptureProfile::Enriched));
        let experiment = injected_rustc_arguments(&request(CaptureProfile::Experiment {
            rustc_arguments: vec!["-C".to_owned(), "target-cpu=native".to_owned()],
        }));

        assert!(
            enriched
                .iter()
                .any(|argument| argument == "symbol-mangling-version=v0")
        );
        assert!(
            enriched
                .iter()
                .any(|argument| argument == "debuginfo=line-tables-only")
        );
        assert!(
            experiment
                .windows(2)
                .any(|pair| pair == ["-C", "target-cpu=native"])
        );
    }

    #[test]
    fn remark_capture_records_arguments_without_forcing_debug_information() {
        let mut request = request(CaptureProfile::Faithful);
        request.capture_remarks = true;

        let arguments = injected_rustc_arguments(&request);

        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["-C", "remark=all"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| { pair[0] == "-Z" && pair[1] == "remark-dir=/analysis/remarks" })
        );
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("debuginfo="))
        );
    }

    #[test]
    fn remark_capture_creates_its_output_directory() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let analysis = temporary.path().join("analysis");
        fs::create_dir(&analysis).expect("the test can create the analysis directory");

        prepare_remarks_directory(&analysis).expect("the remarks directory is valid");

        assert!(analysis.join("remarks").is_dir());
    }

    #[test]
    fn collects_only_optimization_pipeline_remarks() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let nested = temporary.path().join("nested");
        fs::create_dir(&nested).expect("the test can create a nested directory");
        write_remark(&temporary.path().join("crate-b.opt.opt.yaml"), REMARK);
        write_remark(&nested.join("crate-a.opt.opt.yaml"), REMARK);
        write_remark(&temporary.path().join("crate.codegen.opt.yaml"), "not yaml");
        write_remark(&temporary.path().join("unrelated.yaml"), "not yaml");

        let remarks =
            collect_remarks_with_limits(temporary.path(), RemarkCollectionLimits::default())
                .expect("the selected optimization remarks are valid");

        assert_eq!(remarks.len(), 2);
        assert_eq!(remarks[0].name, "crate-b.opt.opt.yaml");
        assert_eq!(remarks[1].name, "nested/crate-a.opt.opt.yaml");
        assert_eq!(
            remarks[0].raw_path.file_name(),
            Some(OsStr::new("crate-b.opt.opt.yaml"))
        );
        assert_eq!(
            remarks[1].raw_path.file_name(),
            Some(OsStr::new("crate-a.opt.opt.yaml"))
        );
        assert_eq!(remarks[0].records[0].pass_name, "inline");
    }

    #[test]
    fn rejects_aggregate_remark_file_and_byte_limits() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        write_remark(&temporary.path().join("a.opt.opt.yaml"), REMARK);
        write_remark(&temporary.path().join("b.opt.opt.yaml"), REMARK);

        let file_error = collect_remarks_with_limits(
            temporary.path(),
            RemarkCollectionLimits {
                max_files: 1,
                ..RemarkCollectionLimits::default()
            },
        )
        .expect_err("two files exceed a one-file collection limit");
        let byte_error = collect_remarks_with_limits(
            temporary.path(),
            RemarkCollectionLimits {
                max_bytes: (REMARK.len() * 2 - 1) as u64,
                ..RemarkCollectionLimits::default()
            },
        )
        .expect_err("the files exceed the aggregate byte limit");
        let record_error = collect_remarks_with_limits(
            temporary.path(),
            RemarkCollectionLimits {
                max_records: 1,
                ..RemarkCollectionLimits::default()
            },
        )
        .expect_err("the files exceed the aggregate record limit");

        assert!(
            file_error
                .to_string()
                .contains("file count exceeds 1, got 2")
        );
        assert!(
            byte_error
                .to_string()
                .contains("aggregate file length exceeds")
        );
        assert!(
            record_error
                .to_string()
                .contains("aggregate record count exceeds 1, got 2")
        );
    }

    #[test]
    fn rejects_a_malformed_selected_remark_file() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        write_remark(
            &temporary.path().join("invalid.opt.opt.yaml"),
            "--- !Passed\nPass: [invalid\n",
        );

        let error =
            collect_remarks_with_limits(temporary.path(), RemarkCollectionLimits::default())
                .expect_err("the selected remark file is malformed");

        assert!(error.to_string().contains("invalid YAML"));
    }

    #[test]
    fn reports_not_captured_without_reading_remark_files() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let remarks = temporary.path().join("remarks");
        fs::create_dir(&remarks).expect("the test can create the remarks directory");
        let mut request = request(CaptureProfile::Faithful);
        request.analysis_directory = temporary.path().to_owned();

        assert_eq!(requested_remarks(&request).unwrap(), None);

        request.capture_remarks = true;
        assert_eq!(requested_remarks(&request).unwrap(), Some(Vec::new()));

        write_remark(&remarks.join("invalid.opt.opt.yaml"), "not yaml");
        assert!(requested_remarks(&request).is_err());
    }

    #[test]
    fn remark_policy_is_part_of_the_serialized_build_request() {
        let without_remarks = request(CaptureProfile::Faithful);
        let mut with_remarks = without_remarks.clone();
        with_remarks.capture_remarks = true;

        assert_ne!(
            serde_json::to_vec(&without_remarks).unwrap(),
            serde_json::to_vec(&with_remarks).unwrap()
        );
    }

    #[test]
    fn preserves_exact_thin_lto_stage_provenance() {
        let provenance = artifact_provenance("example.cgu.0.rcgu.thin-lto-after-import.bc");

        assert_eq!(provenance.compiler_stage, "thin-lto-after-import");
        assert_eq!(provenance.stage, None);
        assert_eq!(provenance.lto, LtoScope::Thin);

        let rename = artifact_provenance("example.cgu.0.rcgu.thin-lto-after-rename.bc");
        assert_eq!(rename.compiler_stage, "thin-lto-after-rename");
        assert_eq!(rename.stage, None);
        assert_eq!(rename.lto, LtoScope::Thin);
        assert_eq!(provenance.codegen_unit.as_deref(), Some("example.cgu.0"));

        let optimized = artifact_provenance("example.cgu.0.rcgu.bc");
        assert_eq!(optimized.stage, Some(LlvmStage::Optimized));
    }

    fn request(capture_profile: CaptureProfile) -> BuildRequest {
        BuildRequest {
            workspace_root: PathBuf::from("/workspace"),
            manifest_path: None,
            package: Some("example".to_owned()),
            target: None,
            profile: Some("release".to_owned()),
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
            target_triple: None,
            locked: false,
            offline: false,
            frozen: false,
            capture_profile,
            capture_remarks: false,
            analysis_directory: PathBuf::from("/analysis"),
        }
    }

    fn write_remark(path: &std::path::Path, contents: &str) {
        fs::write(path, contents).expect("the test can write an optimization-remark fixture");
    }
}
