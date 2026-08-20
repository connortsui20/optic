//! Runs one Cargo analysis and collects supported LLVM modules.
//!
//! [`compile`] uses `cargo rustc` so normal and analysis builds share dependency artifacts. The
//! selected target has a separate Cargo identity because saving compiler temporaries is part of
//! Cargo's fingerprint. [`ingest`] reads the retained artifacts without invoking Cargo again.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::driver::RustcDriver;
use crate::llvm;
use crate::mono;
use crate::toolchain::{CargoContext, inspect_rustc};
use crate::{
    BuildRequest, CaptureProfile, CargoTarget, CompilerInstance, Error, OptimizationRemark,
    RemarkParseLimits, Result, Toolchain, parse_optimization_remarks,
};

const REMARKS_DIRECTORY_NAME: &str = "remarks";
const MAX_REMARK_FILES: usize = 4_096;
const MAX_REMARK_BYTES: u64 = 1024 * 1024 * 1024;

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

/// One disassembled LLVM module and its body index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleEvidence {
    /// The compiler-owned artifact file name.
    pub name: String,

    /// The compiler stage and collection method for this artifact.
    pub provenance: ArtifactProvenance,

    /// The saved LLVM bitcode path.
    pub bitcode_path: PathBuf,

    /// The matching textual LLVM module path.
    pub text_path: PathBuf,

    /// Indexed function definitions in the textual module.
    pub bodies: Vec<BodyRange>,

    /// Indexed function declarations in the textual module.
    pub declarations: Vec<LlvmDeclaration>,

    /// Indexed aliases and their exact direct relationships.
    pub aliases: Vec<LlvmAlias>,
}

/// One raw LLVM optimization-remark file and its parsed records.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemarkEvidence {
    /// The compiler-owned YAML file.
    pub raw_path: PathBuf,

    /// The typed records parsed from the raw YAML document stream.
    pub records: Vec<OptimizationRemark>,
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

/// The unstable-access mechanism used by Optic for one compiler subprocess.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnstableAccessMechanism {
    /// Optic set `RUSTC_BOOTSTRAP=1` for a bounded child process.
    RustcBootstrap,
}

/// One child-process scope in which Optic enables unstable compiler access.
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

/// Unstable compiler access that Optic injected for a capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnstableAccess {
    /// The mechanism used for each recorded scope.
    pub mechanism: UnstableAccessMechanism,

    /// The only child-process scopes that receive the mechanism.
    pub scopes: Vec<UnstableAccessScope>,
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

    /// Unstable compiler access injected by Optic.
    pub unstable_access: UnstableAccess,
}

/// One compiler-artifact event emitted by Cargo.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CargoArtifact {
    /// Cargo's opaque package identifier.
    pub package_id: String,

    /// The target name reported by Cargo.
    pub target_name: String,

    /// The target kinds reported by Cargo.
    pub target_kinds: Vec<String>,

    /// Whether Cargo reported this artifact as fresh.
    pub fresh: bool,
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

        /// The fresh compiler-artifact events reported by Cargo.
        artifacts: Vec<CargoArtifact>,
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

/// All evidence produced by one compiler invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    /// The request and effective compiler invocation.
    pub invocation: CaptureInvocation,

    /// The exact analyzed compiler.
    pub toolchain: Toolchain,

    /// Concrete functions selected by rustc.
    pub instances: Vec<CompilerInstance>,

    /// Supported saved LLVM modules.
    pub modules: Vec<ModuleEvidence>,

    /// Structured LLVM optimization remarks, or `None` when they were not requested.
    pub remarks: Option<Vec<RemarkEvidence>>,
}

/// Runs one analysis for the selected Cargo target.
///
/// # Errors
///
/// Returns an error if the toolchain is unsupported, the analysis directory is not empty, Cargo
/// fails, or emitted LLVM evidence cannot be read.
pub fn compile(request: &BuildRequest) -> Result<CompileOutcome> {
    let cargo = CargoContext::discover(&request.workspace_root)?;
    let toolchain = inspect_rustc(cargo.rustc())?;
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
            scopes: vec![
                UnstableAccessScope::CargoConfigDiscovery,
                UnstableAccessScope::DriverBuild,
                UnstableAccessScope::SelectedTarget,
            ],
        },
    };
    let output = command.output().map_err(|source| Error::StartProcess {
        program: "cargo rustc".to_owned(),
        source,
    })?;
    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: "cargo rustc".to_owned(),
            status: output.status.to_string(),
            diagnostics: cargo_diagnostics(&output.stdout, &output.stderr),
        });
    }

    let cargo_artifacts = cargo_artifacts(&output.stdout);
    if !manifest_path.is_file() {
        if !cargo_artifacts.is_empty() && cargo_artifacts.iter().all(|artifact| artifact.fresh) {
            return Ok(CompileOutcome::Fresh {
                invocation: Box::new(invocation),
                artifacts: cargo_artifacts,
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
    let toolchain = inspect_rustc(cargo.rustc())?;
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
    let output = command.output().map_err(|source| Error::StartProcess {
        program: "cargo rustc freshness check".to_owned(),
        source,
    })?;
    if !output.status.success() {
        let diagnostics = cargo_diagnostics(&output.stdout, &output.stderr);
        if diagnostics.contains(FRESHNESS_STALE_DIAGNOSTIC) {
            return Ok(false);
        }

        return Err(Error::ProcessFailed {
            program: "cargo rustc freshness check".to_owned(),
            status: output.status.to_string(),
            diagnostics,
        });
    }

    let artifacts = cargo_artifacts(&output.stdout);

    Ok(!artifacts.is_empty() && artifacts.iter().all(|artifact| artifact.fresh))
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

/// Reads retained compiler artifacts without invoking Cargo.
///
/// # Errors
///
/// Returns an error if the manifest, LLVM bitcode, or textual LLVM output is invalid.
pub fn ingest(request: &BuildRequest, mut compilation: CompiledCapture) -> Result<EvidenceBundle> {
    require_compiled_evidence(request)?;
    compilation.invocation.request = request.clone();
    let manifest_path = request.analysis_directory.join(mono::MANIFEST_NAME);
    let compiler_manifest = mono::read(&manifest_path, &compilation.toolchain.commit_hash)?;
    compilation.invocation.rustc = Some(compiler_invocation(&compiler_manifest.rustc_arguments)?);
    let modules = supported_bitcode(&request.analysis_directory)?
        .into_iter()
        .map(|artifact| {
            disassemble(
                &compilation.toolchain,
                artifact,
                &request.analysis_directory,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let remarks = requested_remarks(request)?;

    Ok(EvidenceBundle {
        invocation: compilation.invocation,
        toolchain: compilation.toolchain,
        instances: compiler_manifest.instances,
        modules,
        remarks,
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
            | "RUSTC_BOOTSTRAP"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "RUSTFLAGS"
    ) || name.starts_with("CARGO_PROFILE_")
        || name.starts_with("CARGO_TARGET_")
}

fn cargo_artifacts(stdout: &[u8]) -> Vec<CargoArtifact> {
    stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter_map(|message| {
            let package_id = message["package_id"].as_str()?.to_owned();
            let target_name = message["target"]["name"].as_str()?.to_owned();
            let target_kinds = message["target"]["kind"]
                .as_array()?
                .iter()
                .map(|kind| kind.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?;
            let fresh = message["fresh"].as_bool()?;

            Some(CargoArtifact {
                package_id,
                target_name,
                target_kinds,
                fresh,
            })
        })
        .collect()
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

fn cargo_diagnostics(stdout: &[u8], stderr: &[u8]) -> String {
    let mut diagnostics = String::new();

    for message in stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
    {
        let Some(rendered) = message["message"]["rendered"].as_str() else {
            continue;
        };

        diagnostics.push_str(rendered);

        if !rendered.ends_with('\n') {
            diagnostics.push('\n');
        }
    }

    diagnostics.push_str(&String::from_utf8_lossy(stderr));

    if diagnostics.is_empty() {
        diagnostics.push_str(&String::from_utf8_lossy(stdout));
    }

    diagnostics
}

struct BitcodeArtifact {
    path: PathBuf,
    provenance: ArtifactProvenance,
}

#[derive(Clone, Copy)]
struct RemarkCollectionLimits {
    max_files: usize,
    max_bytes: u64,
    parse: RemarkParseLimits,
}

impl Default for RemarkCollectionLimits {
    fn default() -> Self {
        Self {
            max_files: MAX_REMARK_FILES,
            max_bytes: MAX_REMARK_BYTES,
            parse: RemarkParseLimits::default(),
        }
    }
}

fn collect_remarks(directory: &Path) -> Result<Vec<RemarkEvidence>> {
    collect_remarks_with_limits(directory, RemarkCollectionLimits::default())
}

fn requested_remarks(request: &BuildRequest) -> Result<Option<Vec<RemarkEvidence>>> {
    request
        .capture_remarks
        .then(|| collect_remarks(&remarks_directory(&request.analysis_directory)))
        .transpose()
}

fn collect_remarks_with_limits(
    directory: &Path,
    limits: RemarkCollectionLimits,
) -> Result<Vec<RemarkEvidence>> {
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
    paths
        .into_iter()
        .map(|raw_path| {
            let records = parse_optimization_remarks(&raw_path, limits.parse)?;

            Ok(RemarkEvidence { raw_path, records })
        })
        .collect()
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

fn disassemble(
    toolchain: &Toolchain,
    artifact: BitcodeArtifact,
    analysis_directory: &Path,
) -> Result<ModuleEvidence> {
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
    let output = Command::new(&toolchain.llvm_dis)
        .arg("-o")
        .arg(&text_path)
        .arg(&bitcode_path)
        .output()
        .map_err(|source| Error::StartProcess {
            program: toolchain.llvm_dis.display().to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: toolchain.llvm_dis.display().to_string(),
            status: output.status.to_string(),
            diagnostics: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let index = llvm::scan(&text_path)?;

    Ok(ModuleEvidence {
        name,
        provenance,
        bitcode_path,
        text_path,
        bodies: index.bodies,
        declarations: index.declarations,
        aliases: index.aliases,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;

    use super::{
        RemarkCollectionLimits, artifact_provenance, cargo_artifacts, cargo_diagnostics,
        collect_remarks_with_limits, injected_rustc_arguments, prepare_analysis_directory,
        prepare_remarks_directory, requested_remarks,
    };
    use crate::{BuildRequest, CaptureProfile, Error, LlvmStage, LtoScope};

    const REMARK: &str = r#"--- !Passed
Pass: inline
Name: Inlined
Function: _Z8functionv
Args:
  - String: inlined
...
"#;

    #[test]
    fn renders_cargo_json_diagnostics_and_standard_error() {
        let stdout = br#"{"reason":"compiler-message","message":{"rendered":"error: bad input\n"}}
{"reason":"build-finished","success":false}
"#;

        assert_eq!(
            cargo_diagnostics(stdout, b"error: could not compile\n"),
            "error: bad input\nerror: could not compile\n"
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

    #[test]
    fn reads_freshness_from_cargo_artifacts() {
        let stdout = br#"{"reason":"compiler-artifact","package_id":"path+file:///tmp/example#0.1.0","target":{"kind":["lib"],"name":"example"},"fresh":true}
{"reason":"build-finished","success":true}
"#;

        let artifacts = cargo_artifacts(stdout);

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].target_name, "example");
        assert!(artifacts[0].fresh);
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
