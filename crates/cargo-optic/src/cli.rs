//! Implements the human and agent command-line interface.
//!
//! Plain text is the default transport for source and LLVM bodies. `--format json` wraps the same
//! typed application views in a versioned envelope. Read-only commands use explicit capture or
//! instance IDs and never mutate shared navigation state.

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::app::validate_remark_options;
use crate::terminal::{CodeSyntax, Terminal};
use crate::{
    Application, BuildSpec, BuildTarget, CachePolicy, CaptureDetails, CaptureId, CaptureProfile,
    CaptureSummary, CleanSummary, CompareView, CompilerOutput, FindOptions, FindResult, InstanceId,
    InstanceSummary, RemarkEvidenceState, RemarkKindFilter, RemarkOptions, RemarkShowView,
    ShowView, UnstableAccessMechanism, UnstableAccessScope,
};

const MINIMUM_DISPLAY_ID_HEX_DIGITS: usize = 12;
const TRANSPORT_VERSION: u8 = 3;

/// Runs the Cargo Optic CLI and returns its process exit code.
#[must_use]
pub fn run_cli() -> ExitCode {
    let arguments = normalized_arguments();
    let requested_format = Format::from_arguments(&arguments);
    let mut cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            if error.use_stderr() && requested_format == Format::Json {
                return print_json_error("invalid_arguments", &error.to_string());
            }

            let code = if error.use_stderr() { 2 } else { 0 };
            let _ = error.print();

            return ExitCode::from(code);
        }
    };
    let directory = match env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            return print_error(
                cli.command.format(),
                &format!("failed to read the current directory: {error}"),
            );
        }
    };
    if let Some(path) = &mut cli.manifest_path
        && path.is_relative()
    {
        *path = directory.join(&*path);
    }
    if let Command::Clean { format } = &cli.command {
        return finish(execute_clean(
            &directory,
            cli.manifest_path.as_deref(),
            *format,
            cli.color,
        ));
    }
    let mut application = match Application::discover(&directory, cli.manifest_path.as_deref()) {
        Ok(application) => application,
        Err(error) => return print_error(cli.command.format(), &error.to_string()),
    };

    finish(execute(&mut application, cli))
}

fn finish(result: Result<Execution, Failure>) -> ExitCode {
    match result {
        Ok(execution) => match write_stdout(&execution.output) {
            Ok(()) => ExitCode::from(execution.code),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                ExitCode::from(execution.code)
            }
            Err(error) => print_error(Format::Text, &format!("failed to write output: {error}")),
        },
        Err(failure) => print_error(failure.format, &failure.message),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo optic",
    bin_name = "cargo optic",
    version,
    about = "Show source and compiler output from real Cargo builds",
    after_help = concat!(
        "Start with `cargo optic show FUNCTION [CARGO OPTIONS]`.\n",
        "Use `cargo optic help show` for all inspection modes."
    )
)]
struct Cli {
    /// Uses the specified Cargo manifest.
    #[arg(long, global = true, value_name = "PATH")]
    manifest_path: Option<PathBuf>,

    /// Controls ANSI color and syntax highlighting.
    #[arg(long, global = true, value_enum, default_value_t)]
    color: ColorChoice,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Captures or reuses compiler evidence for one Cargo target.
    #[command(after_long_help = "Example:\n  cargo optic capture -p my-crate --lib --release")]
    Capture {
        /// The Cargo build and cache options.
        #[command(flatten)]
        build: BuildOptions,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Lists the completed captures in this workspace.
    Captures {
        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Shows the exact request, invocation, and artifacts for one capture.
    Inspect {
        /// Selects a capture by its full ID or a unique prefix.
        #[arg(long)]
        capture: CaptureId,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Shows the size and object counts of the evidence store.
    Status {
        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Removes one capture and retains shared blobs until garbage collection.
    Remove {
        /// Selects a capture by its full ID or a unique prefix.
        #[arg(long)]
        capture: CaptureId,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Removes blobs that no completed capture references.
    Gc {
        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Verifies every blob referenced by completed captures.
    Verify {
        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Compares compact LLVM structure for two exact instances.
    Compare {
        /// Selects the first instance by full ID or unique prefix.
        #[arg(long)]
        before: InstanceId,

        /// Selects the second instance by full ID or unique prefix.
        #[arg(long)]
        after: InstanceId,

        /// Selects one compiler output.
        #[arg(long, value_enum, default_value_t)]
        output: CompilerOutput,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Removes all stored Optic evidence for this workspace.
    #[command(after_long_help = concat!(
        "Example:\n  cargo optic clean\n\n",
        "This command keeps `.optic` configuration and locks. ",
        "It does not remove the Cargo target directory."
    ))]
    Clean {
        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Finds concrete compiler instances in one capture.
    #[command(after_long_help = "Example:\n  cargo optic find --capture cap_01a0 my_crate::kernel")]
    Find {
        /// Selects a capture by its full ID or a unique prefix.
        #[arg(long)]
        capture: CaptureId,

        /// Matches a definition or concrete compiler instance.
        query: String,

        /// Restricts results to one compiler crate.
        #[arg(long = "crate")]
        crate_name: Option<String>,

        /// Restricts results to one qualified definition path.
        #[arg(long)]
        definition: Option<String>,

        /// Requires a standalone definition in one compiler output.
        #[arg(long, value_enum)]
        available: Option<CompilerOutput>,

        /// Limits the number of returned instances.
        #[arg(
            long,
            default_value_t = FindOptions::DEFAULT_LIMIT,
            value_parser = parse_find_limit
        )]
        limit: usize,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Shows one compiler output for one concrete instance.
    ///
    /// A query without `--capture` builds the selected Cargo target. A query with `--capture`
    /// searches existing evidence. `--instance` directly selects an instance and its capture.
    #[command(after_long_help = concat!(
        "Examples:\n",
        "  Build and show optimized LLVM:\n",
        "    cargo optic show my_crate::kernel -p my-crate --lib --release\n\n",
        "  Search a completed capture:\n",
        "    cargo optic show my_crate::kernel --capture cap_01a0\n\n",
        "  Show one exact instance and its source:\n",
        "    cargo optic show --instance ins_01a0 --source\n\n",
        "  Show pre-optimization LLVM:\n",
        "    cargo optic show --instance ins_01a0 --output llvm-pre-opt"
    ))]
    Show {
        /// Builds and searches, or searches the capture selected by `--capture`.
        query: Option<String>,

        /// Searches stored evidence by a full capture ID or unique prefix.
        #[arg(long)]
        capture: Option<CaptureId>,

        /// Selects stored evidence by a full instance ID or unique prefix.
        ///
        /// This option does not need a capture ID.
        #[arg(long)]
        instance: Option<InstanceId>,

        /// Selects one compiler output or LLVM optimization remarks.
        #[arg(long, value_enum, default_value_t)]
        output: ShowOutput,

        /// Restricts optimization remarks to one category.
        #[arg(long, value_enum)]
        kind: Option<RemarkKindFilter>,

        /// Restricts optimization remarks to one exact LLVM pass name.
        #[arg(long = "pass")]
        pass_name: Option<String>,

        /// Limits returned optimization remarks.
        #[arg(long, value_parser = parse_remark_limit)]
        limit: Option<usize>,

        /// Includes the captured Rust source item.
        #[arg(long)]
        source: bool,

        /// The Cargo build and cache options.
        #[command(flatten)]
        build: BuildOptions,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },
}

impl Command {
    const fn format(&self) -> Format {
        match self {
            Self::Capture { format, .. }
            | Self::Captures { format }
            | Self::Inspect { format, .. }
            | Self::Status { format }
            | Self::Remove { format, .. }
            | Self::Gc { format }
            | Self::Verify { format }
            | Self::Compare { format, .. }
            | Self::Clean { format }
            | Self::Find { format, .. }
            | Self::Show { format, .. } => *format,
        }
    }
}

#[derive(Clone, Debug, Default, Args)]
struct BuildOptions {
    /// Requests new compiler evidence after pending-ingestion recovery.
    #[arg(long)]
    fresh: bool,

    /// Selects one Cargo package.
    #[arg(short = 'p', long = "package")]
    package: Option<String>,

    /// Selects the package library.
    #[arg(
        long,
        conflicts_with_all = ["bin", "bench", "example"]
    )]
    lib: bool,

    /// Selects a named binary.
    #[arg(long, conflicts_with_all = ["lib", "bench", "example"])]
    bin: Option<String>,

    /// Selects a named benchmark.
    #[arg(long, conflicts_with_all = ["lib", "bin", "example"])]
    bench: Option<String>,

    /// Selects a named example.
    #[arg(long, conflicts_with_all = ["lib", "bin", "bench"])]
    example: Option<String>,

    /// Uses the Cargo release profile.
    #[arg(long, conflicts_with = "profile")]
    release: bool,

    /// Uses a named Cargo profile.
    #[arg(long)]
    profile: Option<String>,

    /// Enables a comma-separated list of Cargo features.
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    /// Enables all Cargo features.
    #[arg(long)]
    all_features: bool,

    /// Disables default Cargo features.
    #[arg(long)]
    no_default_features: bool,

    /// Compiles for the specified target triple.
    #[arg(long)]
    target: Option<String>,

    /// Requires an unchanged Cargo lock file.
    #[arg(long)]
    locked: bool,

    /// Prevents Cargo network access.
    #[arg(long)]
    offline: bool,

    /// Enables Cargo locked and offline behavior.
    #[arg(long)]
    frozen: bool,

    /// Controls whether Optic preserves or enriches compiler settings.
    #[arg(long, value_enum, default_value_t)]
    evidence_profile: CaptureProfile,

    /// Passes one compiler argument to an experiment capture.
    #[arg(long = "rustc-arg")]
    rustc_arguments: Vec<String>,

    /// Captures LLVM optimization remarks for the selected target.
    ///
    /// Use `--evidence-profile enriched` when source locations are needed.
    #[arg(long)]
    remarks: bool,
}

impl BuildOptions {
    const fn cache_policy(&self) -> CachePolicy {
        if self.fresh {
            CachePolicy::Refresh
        } else {
            CachePolicy::Reuse
        }
    }

    fn to_spec(&self, manifest_path: Option<PathBuf>) -> BuildSpec {
        let target = if self.lib {
            Some(BuildTarget::Library)
        } else if let Some(name) = &self.bin {
            Some(BuildTarget::Binary(name.clone()))
        } else if let Some(name) = &self.bench {
            Some(BuildTarget::Benchmark(name.clone()))
        } else {
            self.example
                .as_ref()
                .map(|name| BuildTarget::Example(name.clone()))
        };
        let profile = if self.release {
            Some("release".to_owned())
        } else {
            self.profile.clone()
        };

        BuildSpec {
            manifest_path,
            package: self.package.clone(),
            target,
            profile,
            features: self.features.clone(),
            all_features: self.all_features,
            no_default_features: self.no_default_features,
            target_triple: self.target.clone(),
            locked: self.locked,
            offline: self.offline,
            frozen: self.frozen,
            capture_profile: self.evidence_profile,
            rustc_arguments: self.rustc_arguments.clone(),
            capture_remarks: self.remarks,
        }
    }

    fn has_build_selection(&self) -> bool {
        self.package.is_some()
            || self.fresh
            || self.lib
            || self.bin.is_some()
            || self.bench.is_some()
            || self.example.is_some()
            || self.release
            || self.profile.is_some()
            || !self.features.is_empty()
            || self.all_features
            || self.no_default_features
            || self.target.is_some()
            || self.locked
            || self.offline
            || self.frozen
            || self.evidence_profile != CaptureProfile::Faithful
            || !self.rustc_arguments.is_empty()
            || self.remarks
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum ShowOutput {
    #[default]
    #[value(name = "llvm")]
    Llvm,

    #[value(name = "llvm-pre-opt")]
    LlvmPreOpt,

    Remarks,
}

impl ShowOutput {
    const fn compiler_output(self) -> Option<CompilerOutput> {
        match self {
            Self::Llvm => Some(CompilerOutput::Llvm),
            Self::LlvmPreOpt => Some(CompilerOutput::LlvmPreOpt),
            Self::Remarks => None,
        }
    }
}

impl std::fmt::Display for ShowOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Llvm => "llvm",
            Self::LlvmPreOpt => "llvm-pre-opt",
            Self::Remarks => "remarks",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
}

impl Format {
    fn from_arguments(arguments: &[OsString]) -> Self {
        let requests_json = arguments.windows(2).any(|arguments| {
            arguments[0] == "--format" && arguments[1].as_encoded_bytes() == b"json"
        }) || arguments.iter().any(|argument| {
            argument
                .as_encoded_bytes()
                .strip_prefix(b"--format=")
                .is_some_and(|value| value == b"json")
        });

        if requests_json {
            Self::Json
        } else {
            Self::Text
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    fn enabled(self, format: Format) -> bool {
        if format == Format::Json {
            return false;
        }

        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => {
                io::stdout().is_terminal()
                    && env::var_os("NO_COLOR").is_none()
                    && env::var_os("TERM").is_none_or(|term| term != "dumb")
            }
        }
    }
}

struct Execution {
    code: u8,
    output: String,
}

#[derive(Serialize)]
struct SuccessEnvelope<'a, T> {
    version: u8,
    ok: bool,
    result: &'a T,
}

#[derive(Serialize)]
struct SelectionEnvelope<'a, T> {
    version: u8,
    ok: bool,
    error: SelectionEnvelopeError<'a, T>,
}

#[derive(Serialize)]
struct SelectionEnvelopeError<'a, T> {
    code: &'static str,
    message: &'static str,
    result: &'a T,
}

#[derive(Serialize)]
struct OperationErrorEnvelope<'a> {
    version: u8,
    ok: bool,
    error: OperationError<'a>,
}

#[derive(Serialize)]
struct OperationError<'a> {
    code: &'static str,
    message: &'a str,
}

struct Failure {
    format: Format,
    message: String,
}

/// A validated identifier prepared for plain-text output.
struct DisplayIdentifier {
    /// The fixed-width or collision-expanded prefix shown to the user.
    text: String,

    /// The byte length that uniquely identifies the full ASCII identifier.
    unique_prefix_length: usize,
}

#[derive(Clone, Copy)]
enum SelectionFailure {
    NotFound,
    Ambiguous,
}

impl SelectionFailure {
    const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
        }
    }
}

impl DisplayIdentifier {
    fn new(full: &str, unique_prefix: &str) -> Self {
        assert!(
            full.starts_with(unique_prefix),
            "full ID must start with its unique prefix, got {full} and {unique_prefix}"
        );
        let type_prefix_length = full
            .find('_')
            .map(|index| index + 1)
            .expect("validated IDs contain a type prefix separator");
        let minimum_length = (type_prefix_length + MINIMUM_DISPLAY_ID_HEX_DIGITS).min(full.len());
        let display_length = minimum_length.max(unique_prefix.len());

        Self {
            text: full[..display_length].to_owned(),
            unique_prefix_length: unique_prefix.len(),
        }
    }

    fn full(identifier: &str) -> Self {
        Self::new(identifier, identifier)
    }
}

struct ShowRequest {
    manifest_path: Option<PathBuf>,

    query: Option<String>,

    capture: Option<CaptureId>,

    instance: Option<InstanceId>,

    output: ShowOutput,

    remark_options: RemarkOptions,

    remark_filters_supplied: bool,

    include_source: bool,

    build: BuildOptions,

    format: Format,

    color: ColorChoice,
}

fn execute_clean(
    directory: &Path,
    manifest_path: Option<&Path>,
    format: Format,
    color: ColorChoice,
) -> Result<Execution, Failure> {
    let terminal = Terminal::new(color.enabled(format));
    let summary = Application::clean(directory, manifest_path).map_err(|error| Failure {
        format,
        message: error.to_string(),
    })?;

    success(format, &summary, clean_text(&summary, &terminal))
}

fn execute(application: &mut Application, cli: Cli) -> Result<Execution, Failure> {
    let manifest_path = cli.manifest_path;
    let color = cli.color;

    match cli.command {
        Command::Capture { build, format } => {
            let terminal = Terminal::new(color.enabled(format));
            write_progress("Resolving compiler evidence...");
            let spec = build.to_spec(manifest_path);
            let summary = application
                .capture_with_events(&spec, build.cache_policy(), write_cargo_event)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;
            let display_id =
                display_capture_id(application, &summary.id, format).map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;

            success(
                format,
                &summary,
                capture_text(&summary, &display_id, &terminal),
            )
        }
        Command::Captures { format } => {
            let terminal = Terminal::new(color.enabled(format));
            let captures = application.captures().map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
            let display_ids = captures
                .iter()
                .map(|capture| display_capture_id(application, &capture.id, format))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;

            success(
                format,
                &captures,
                captures_text(&captures, &display_ids, &terminal),
            )
        }
        Command::Inspect { capture, format } => {
            let terminal = Terminal::new(color.enabled(format));
            let details = application.inspect(&capture).map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
            let display_id =
                display_capture_id(application, &details.summary.id, format).map_err(|error| {
                    Failure {
                        format,
                        message: error.to_string(),
                    }
                })?;

            success(
                format,
                &details,
                capture_details_text(&details, &display_id, &terminal),
            )
        }
        Command::Status { format } => {
            let status = application.status().map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
            success(
                format,
                &status,
                format!(
                    "{} captures, {} blobs, {} bytes, {} pending captures, {} pending bytes\n",
                    status.captures,
                    status.blobs,
                    status.blob_bytes,
                    status.pending,
                    status.pending_bytes
                ),
            )
        }
        Command::Remove { capture, format } => {
            let summary = application.remove(&capture).map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
            success(
                format,
                &summary,
                format!(
                    "Removed capture {}. Run `cargo optic gc` to reclaim blobs.\n",
                    summary.capture_id
                ),
            )
        }
        Command::Gc { format } => {
            let summary = application.gc().map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
            success(
                format,
                &summary,
                format!(
                    "Removed {} unreferenced blobs ({} bytes).\n",
                    summary.removed_blobs, summary.removed_bytes
                ),
            )
        }
        Command::Verify { format } => {
            let summary = application.verify().map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
            success(
                format,
                &summary,
                format!(
                    "Verified {} blobs ({} bytes).\n",
                    summary.verified_blobs, summary.verified_bytes
                ),
            )
        }
        Command::Compare {
            before,
            after,
            output,
            format,
        } => {
            let comparison = application
                .compare(&before, &after, output)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;
            success(format, &comparison, compare_text(&comparison))
        }
        Command::Clean { .. } => {
            unreachable!("clean executes before the application opens its store")
        }
        Command::Find {
            capture,
            query,
            crate_name,
            definition,
            available,
            limit,
            format,
        } => {
            let terminal = Terminal::new(color.enabled(format));
            let options = FindOptions {
                query,
                crate_name,
                definition,
                available,
                limit,
            };
            let result = application
                .find(&capture, &options)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;
            let display_capture = display_capture_id(application, &result.capture_id, format)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;
            let display_instances = display_instance_ids(application, &result.instances, format)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;

            success(
                format,
                &result,
                find_text(&result, &display_capture, &display_instances, &terminal),
            )
        }
        Command::Show {
            query,
            capture,
            instance,
            output,
            kind,
            pass_name,
            limit,
            source,
            build,
            format,
        } => {
            let remark_filters_supplied = kind.is_some() || pass_name.is_some() || limit.is_some();
            execute_show(
                application,
                ShowRequest {
                    manifest_path,
                    query,
                    capture,
                    instance,
                    output,
                    remark_options: RemarkOptions {
                        kind,
                        pass: pass_name,
                        limit: limit.unwrap_or(RemarkOptions::DEFAULT_LIMIT),
                    },
                    remark_filters_supplied,
                    include_source: source,
                    build,
                    format,
                    color,
                },
            )
        }
    }
}

fn execute_show(application: &mut Application, request: ShowRequest) -> Result<Execution, Failure> {
    let ShowRequest {
        manifest_path,
        query,
        capture,
        instance,
        output,
        remark_options,
        remark_filters_supplied,
        include_source,
        build,
        format,
        color,
    } = request;
    let terminal = Terminal::new(color.enabled(format));
    if output == ShowOutput::Remarks {
        validate_remark_options(&remark_options).map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;
    } else if remark_filters_supplied {
        return Err(Failure {
            format,
            message: "--kind, --pass, and --limit require --output remarks".to_owned(),
        });
    }

    if let Some(instance) = instance {
        if capture.is_some() {
            return Err(Failure {
                format,
                message: "--instance cannot be combined with --capture, got both options"
                    .to_owned(),
            });
        }
        if query.is_some() {
            return Err(Failure {
                format,
                message: "--instance cannot be combined with a query, got both selections"
                    .to_owned(),
            });
        }
        if build.has_build_selection() {
            return Err(Failure {
                format,
                message:
                    "--instance cannot be combined with Cargo build options, got both selections"
                        .to_owned(),
            });
        }

        return show_selected(
            application,
            &instance,
            output,
            &remark_options,
            include_source,
            format,
            &terminal,
        );
    }

    let Some(query) = query else {
        let message = if capture.is_some() {
            "--capture requires a query, got no query"
        } else {
            "show requires a query or --instance INSTANCE, got neither"
        };

        return Err(Failure {
            format,
            message: message.to_owned(),
        });
    };

    let capture = if let Some(capture) = capture {
        if build.has_build_selection() {
            return Err(Failure {
                format,
                message:
                    "--capture cannot be combined with Cargo build options, got both selections"
                        .to_owned(),
            });
        }

        capture
    } else {
        write_progress("Resolving compiler evidence...");
        let mut spec = build.to_spec(manifest_path);
        if output == ShowOutput::Remarks {
            spec.capture_remarks = true;
        }
        application
            .capture_with_events(&spec, build.cache_policy(), write_cargo_event)
            .map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?
            .id
    };
    let result = application
        .find(&capture, &FindOptions::new(query))
        .map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;

    select_and_show(
        application,
        &result,
        output,
        &remark_options,
        include_source,
        format,
        &terminal,
    )
}

fn select_and_show(
    application: &Application,
    result: &FindResult,
    output: ShowOutput,
    remark_options: &RemarkOptions,
    include_source: bool,
    format: Format,
    terminal: &Terminal,
) -> Result<Execution, Failure> {
    let [instance] = result.instances.as_slice() else {
        let failure = if result.instances.is_empty() {
            SelectionFailure::NotFound
        } else {
            SelectionFailure::Ambiguous
        };
        let display_capture =
            display_capture_id(application, &result.capture_id, format).map_err(|error| {
                Failure {
                    format,
                    message: error.to_string(),
                }
            })?;
        let display_instances = display_instance_ids(application, &result.instances, format)
            .map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
        let text = selection_text(
            result,
            &display_capture,
            &display_instances,
            output,
            remark_options,
            include_source,
            terminal,
        );

        return selection(format, result, failure, text);
    };

    show_selected(
        application,
        &instance.id,
        output,
        remark_options,
        include_source,
        format,
        terminal,
    )
}

fn show_selected(
    application: &Application,
    instance: &InstanceId,
    output: ShowOutput,
    remark_options: &RemarkOptions,
    include_source: bool,
    format: Format,
    terminal: &Terminal,
) -> Result<Execution, Failure> {
    if let Some(compiler_output) = output.compiler_output() {
        let view = application
            .show(instance, compiler_output, include_source)
            .map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
        let display_capture =
            display_capture_id(application, &view.capture_id, format).map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
        let display_instance = display_instance_id(application, &view.instance.id, format)
            .map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;

        return success(
            format,
            &view,
            show_text(&view, &display_capture, &display_instance, terminal),
        );
    }

    let view = application
        .show_remarks(instance, remark_options, include_source)
        .map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;
    let display_capture =
        display_capture_id(application, &view.capture_id, format).map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;
    let display_instance =
        display_instance_id(application, &view.instance.id, format).map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;

    success(
        format,
        &view,
        remark_show_text(&view, &display_capture, &display_instance, terminal),
    )
}

fn display_instance_ids(
    application: &Application,
    instances: &[InstanceSummary],
    format: Format,
) -> crate::Result<Vec<DisplayIdentifier>> {
    instances
        .iter()
        .map(|instance| display_instance_id(application, &instance.id, format))
        .collect()
}

fn display_capture_id(
    application: &Application,
    capture_id: &CaptureId,
    format: Format,
) -> crate::Result<DisplayIdentifier> {
    match format {
        Format::Text => {
            let unique_prefix = application.unique_capture_prefix(capture_id)?;

            Ok(DisplayIdentifier::new(
                capture_id.as_str(),
                unique_prefix.as_str(),
            ))
        }
        Format::Json => Ok(DisplayIdentifier::full(capture_id.as_str())),
    }
}

fn display_instance_id(
    application: &Application,
    instance_id: &InstanceId,
    format: Format,
) -> crate::Result<DisplayIdentifier> {
    match format {
        Format::Text => {
            let unique_prefix = application.unique_instance_prefix(instance_id)?;

            Ok(DisplayIdentifier::new(
                instance_id.as_str(),
                unique_prefix.as_str(),
            ))
        }
        Format::Json => Ok(DisplayIdentifier::full(instance_id.as_str())),
    }
}

fn success<T: Serialize>(format: Format, result: &T, text: String) -> Result<Execution, Failure> {
    let output = match format {
        Format::Text => text,
        Format::Json => {
            let envelope = SuccessEnvelope {
                version: TRANSPORT_VERSION,
                ok: true,
                result,
            };

            json_output(format, &envelope)?
        }
    };

    Ok(Execution { code: 0, output })
}

fn selection<T: Serialize>(
    format: Format,
    result: &T,
    failure: SelectionFailure,
    text: String,
) -> Result<Execution, Failure> {
    let output = match format {
        Format::Text => text,
        Format::Json => {
            let envelope = SelectionEnvelope {
                version: TRANSPORT_VERSION,
                ok: false,
                error: SelectionEnvelopeError {
                    code: failure.code(),
                    message: "the query must match exactly one compiler instance",
                    result,
                },
            };

            json_output(format, &envelope)?
        }
    };

    Ok(Execution { code: 2, output })
}

fn json_output<T: Serialize>(format: Format, value: &T) -> Result<String, Failure> {
    let mut output = serde_json::to_string_pretty(value).map_err(|error| Failure {
        format,
        message: format!("failed to encode JSON output: {error}"),
    })?;
    output.push('\n');

    Ok(output)
}

fn print_error(format: Format, message: &str) -> ExitCode {
    let _ = match format {
        Format::Text => write_stderr(&format!("error: {message}\n")),
        Format::Json => {
            let envelope = OperationErrorEnvelope {
                version: TRANSPORT_VERSION,
                ok: false,
                error: OperationError {
                    code: "operation_failed",
                    message,
                },
            };
            let output = serde_json::to_string_pretty(&envelope)
                .expect("operation error envelopes contain only strings and primitive values");

            write_stdout(&format!("{output}\n"))
        }
    };

    ExitCode::FAILURE
}

fn print_json_error(code: &'static str, message: &str) -> ExitCode {
    let envelope = OperationErrorEnvelope {
        version: TRANSPORT_VERSION,
        ok: false,
        error: OperationError { code, message },
    };
    let output = serde_json::to_string_pretty(&envelope)
        .expect("operation error envelopes contain only strings and primitive values");
    let _ = write_stdout(&format!("{output}\n"));

    ExitCode::from(2)
}

fn parse_find_limit(value: &str) -> std::result::Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| format!("find limit must be an integer, got {value}"))?;
    if !(1..=FindOptions::MAX_LIMIT).contains(&limit) {
        return Err(format!(
            "find limit must be from 1 through {}, got {limit}",
            FindOptions::MAX_LIMIT
        ));
    }

    Ok(limit)
}

fn parse_remark_limit(value: &str) -> std::result::Result<usize, String> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| format!("remark limit must be an integer, got {value}"))?;
    if !(1..=RemarkOptions::MAX_LIMIT).contains(&limit) {
        return Err(format!(
            "remark limit must be from 1 through {}, got {limit}",
            RemarkOptions::MAX_LIMIT
        ));
    }

    Ok(limit)
}

fn write_progress(message: &str) {
    let _ = write_stderr(&format!("{message}\n"));
}

fn write_cargo_event(event: crate::CargoProcessEvent) {
    let _ = io::stderr().lock().write_all(event.bytes());
}

fn write_stdout(output: &str) -> io::Result<()> {
    io::stdout().lock().write_all(output.as_bytes())
}

fn write_stderr(output: &str) -> io::Result<()> {
    io::stderr().lock().write_all(output.as_bytes())
}

fn capture_text(
    summary: &CaptureSummary,
    display_id: &DisplayIdentifier,
    terminal: &Terminal,
) -> String {
    let status = match summary.disposition {
        crate::CaptureDisposition::Captured => "captured",
        crate::CaptureDisposition::Reused => "reused",
        crate::CaptureDisposition::Resumed => "resumed",
    };
    let find = terminal.command_with_identifier(
        "cargo optic find --capture ",
        &display_id.text,
        display_id.unique_prefix_length,
        " QUERY",
    );
    let show = terminal.command_with_identifier(
        "cargo optic show QUERY --capture ",
        &display_id.text,
        display_id.unique_prefix_length,
        "",
    );

    format!(
        "{} {}\n{}{}\n{}{}\n{}{}\n{}{}\n{}{}\n\n{}\n  {}\n  {}\n",
        terminal.heading("Capture"),
        terminal.identifier(&display_id.text, display_id.unique_prefix_length),
        terminal.label("  Status     "),
        terminal.positive(status),
        terminal.label("  Toolchain  "),
        format_args!(
            "rustc {} · LLVM {}",
            summary.rustc_release, summary.llvm_version
        ),
        terminal.label("  Target     "),
        summary.target,
        terminal.label("  Instances  "),
        summary.instance_count,
        terminal.label("  Remarks    "),
        remark_capture_text(summary),
        terminal.heading("Next commands"),
        find,
        show,
    )
}

fn clean_text(summary: &CleanSummary, terminal: &Terminal) -> String {
    if summary.removed {
        format!(
            "{} at {}.\n",
            terminal.positive("Removed stored Optic evidence"),
            summary.path.display(),
        )
    } else {
        format!(
            "{} at {}.\n",
            terminal.warning("No stored Optic evidence exists"),
            summary.path.display(),
        )
    }
}

fn captures_text(
    captures: &[CaptureSummary],
    display_ids: &[DisplayIdentifier],
    terminal: &Terminal,
) -> String {
    if captures.is_empty() {
        return format!("{}\n", terminal.warning("No captures."));
    }

    let mut output = format!("{}\n", terminal.heading("Captures"));
    for (capture, display_id) in captures.iter().zip(display_ids) {
        writeln!(
            output,
            "{}  {}  {}  {:?}  {} instances, {} artifacts, {}",
            terminal.identifier(&display_id.text, display_id.unique_prefix_length),
            capture.rustc_release,
            capture.target,
            capture.capture_profile,
            capture.instance_count,
            capture.module_count,
            remark_capture_text(capture),
        )
        .expect("writing capture text to a String cannot fail");
    }

    output
}

fn capture_details_text(
    details: &CaptureDetails,
    display_id: &DisplayIdentifier,
    terminal: &Terminal,
) -> String {
    let mut output = format!(
        concat!(
            "{}\n",
            "  Capture   {}\n",
            "  Profile   {:?}\n",
            "  Target    {}\n",
            "  Compiler  {}\n",
            "  Release   {}\n",
            "  Commit    {}\n",
            "  Host      {}\n",
            "  LLVM      {}\n",
            "  Sysroot   {}\n",
            "  llvm-dis  {}\n",
            "  Cargo     {} {}\n",
        ),
        terminal.heading("Capture details"),
        terminal.identifier(&display_id.text, display_id.unique_prefix_length),
        details.summary.capture_profile,
        details.summary.target,
        details.compiler.rustc.display(),
        details.compiler.release,
        details.compiler.commit_hash,
        details.compiler.host,
        details.compiler.llvm_version,
        details.compiler.sysroot.display(),
        details.compiler.llvm_dis.display(),
        details.cargo.program,
        details.cargo.arguments.join(" "),
    );
    if let Some(rustc) = &details.rustc {
        writeln!(
            output,
            "  rustc    {} {}",
            rustc.program,
            rustc.arguments.join(" ")
        )
        .expect("writing capture details to a String cannot fail");
    }
    let authorized_scopes = details
        .unstable_access
        .authorized_scopes
        .iter()
        .map(|scope| unstable_access_scope_name(*scope))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        output,
        "  Unstable  {} (authorized: {})",
        unstable_access_mechanism_name(details.unstable_access.mechanism),
        authorized_scopes,
    )
    .expect("writing capture details to a String cannot fail");
    writeln!(
        output,
        "  Artifacts  {} ({} instances)",
        details.artifacts.len(),
        details.summary.instance_count
    )
    .expect("writing capture details to a String cannot fail");
    for artifact in &details.artifacts {
        writeln!(
            output,
            "    {}  {}  {} definitions, {} declarations, {} aliases",
            artifact.name,
            artifact.compiler_stage,
            artifact.definitions,
            artifact.declarations,
            artifact.aliases,
        )
        .expect("writing capture details to a String cannot fail");
    }
    writeln!(
        output,
        "  Remarks  {}",
        remark_capture_text(&details.summary)
    )
    .expect("writing capture details to a String cannot fail");
    for file in &details.remark_files {
        writeln!(output, "    {}  {} records", file.name, file.records)
            .expect("writing capture details to a String cannot fail");
    }

    output
}

fn remark_capture_text(summary: &CaptureSummary) -> String {
    match summary.remarks.state {
        RemarkEvidenceState::NotCaptured => "remarks not captured".to_owned(),
        RemarkEvidenceState::CapturedEmpty => {
            format!(
                "remarks captured, {} files, no records",
                summary.remarks.files
            )
        }
        RemarkEvidenceState::Captured => format!(
            "{} remark records ({} linked, {} unlinked)",
            summary.remarks.records,
            summary.remarks.linked_records,
            summary.remarks.unlinked_records,
        ),
    }
}

fn unstable_access_mechanism_name(mechanism: UnstableAccessMechanism) -> &'static str {
    match mechanism {
        UnstableAccessMechanism::RustcBootstrap => "rustc-bootstrap",
    }
}

fn unstable_access_scope_name(scope: UnstableAccessScope) -> &'static str {
    match scope {
        UnstableAccessScope::CargoConfigDiscovery => "cargo-config-discovery",
        UnstableAccessScope::DriverBuild => "driver-build",
        UnstableAccessScope::SelectedTarget => "selected-target",
    }
}

fn compare_text(comparison: &CompareView) -> String {
    let compatibility = if comparison.compatibility_differences.is_empty() {
        "compatible capture dimensions".to_owned()
    } else {
        format!(
            "different capture dimensions: {}",
            comparison.compatibility_differences.join(", ")
        )
    };

    format!(
        "Comparison ({}, {})\n  Bodies                 {} -> {} ({:+})\n  Bytes                  {} -> {} ({:+})\n  Instructions           {} -> {} ({:+})\n  Vector lines           {} -> {} ({:+})\n  Call sites             {} -> {} ({:+})\n  Direct runtime calls   {} -> {} ({:+})\n  Indirect calls         {} -> {} ({:+})\n  Inline assembly        {} -> {} ({:+})\n  Memory intrinsics      {} -> {} ({:+})\n  Assumption intrinsics  {} -> {} ({:+})\n  Lifetime intrinsics    {} -> {} ({:+})\n  Metadata intrinsics    {} -> {} ({:+})\n  Other intrinsics       {} -> {} ({:+})\n  Safety checks          {} -> {} ({:+})\n",
        comparison.output,
        compatibility,
        comparison.before.bodies,
        comparison.after.bodies,
        comparison.delta.bodies,
        comparison.before.bytes,
        comparison.after.bytes,
        comparison.delta.bytes,
        comparison.before.instructions,
        comparison.after.instructions,
        comparison.delta.instructions,
        comparison.before.vector_lines,
        comparison.after.vector_lines,
        comparison.delta.vector_lines,
        comparison.before.call_sites.total,
        comparison.after.call_sites.total,
        comparison.delta.call_sites.total,
        comparison.before.call_sites.direct_non_intrinsic,
        comparison.after.call_sites.direct_non_intrinsic,
        comparison.delta.call_sites.direct_non_intrinsic,
        comparison.before.call_sites.indirect,
        comparison.after.call_sites.indirect,
        comparison.delta.call_sites.indirect,
        comparison.before.call_sites.inline_asm,
        comparison.after.call_sites.inline_asm,
        comparison.delta.call_sites.inline_asm,
        comparison.before.call_sites.memory_intrinsics,
        comparison.after.call_sites.memory_intrinsics,
        comparison.delta.call_sites.memory_intrinsics,
        comparison.before.call_sites.assumption_intrinsics,
        comparison.after.call_sites.assumption_intrinsics,
        comparison.delta.call_sites.assumption_intrinsics,
        comparison.before.call_sites.lifetime_intrinsics,
        comparison.after.call_sites.lifetime_intrinsics,
        comparison.delta.call_sites.lifetime_intrinsics,
        comparison.before.call_sites.metadata_only_intrinsics,
        comparison.after.call_sites.metadata_only_intrinsics,
        comparison.delta.call_sites.metadata_only_intrinsics,
        comparison.before.call_sites.other_intrinsics,
        comparison.after.call_sites.other_intrinsics,
        comparison.delta.call_sites.other_intrinsics,
        comparison.before.safety_checks,
        comparison.after.safety_checks,
        comparison.delta.safety_checks,
    )
}

fn find_text(
    result: &FindResult,
    display_capture: &DisplayIdentifier,
    display_instances: &[DisplayIdentifier],
    terminal: &Terminal,
) -> String {
    if result.instances.is_empty() {
        return format!(
            "{} {}.\n",
            terminal.warning("No matching instances in"),
            terminal.identifier(&display_capture.text, display_capture.unique_prefix_length),
        );
    }

    let mut output = format!(
        "{} {}\n",
        terminal.heading("Capture"),
        terminal.identifier(&display_capture.text, display_capture.unique_prefix_length),
    );
    for (instance, display_id) in result.instances.iter().zip(display_instances) {
        output.push_str(&instance_text(
            instance,
            display_id,
            duplicate_display_name(result, instance),
            terminal,
        ));
        writeln!(
            output,
            "  {}",
            show_command(terminal, display_id, ShowOutput::default(), None, false)
        )
        .expect("writing instance text to a String cannot fail");
    }
    if result.truncated {
        writeln!(
            output,
            "{}",
            terminal.warning("More matching instances exist. Increase --limit to show them.")
        )
        .expect("writing find text to a String cannot fail");
    }

    output
}

fn instance_text(
    instance: &InstanceSummary,
    display_id: &DisplayIdentifier,
    disambiguate: bool,
    terminal: &Terminal,
) -> String {
    let optimized = instance
        .availability
        .iter()
        .find(|availability| availability.output == CompilerOutput::Llvm)
        .is_some_and(crate::OutputAvailability::has_definition);
    let pre_optimization = instance
        .availability
        .iter()
        .find(|availability| availability.output == CompilerOutput::LlvmPreOpt)
        .is_some_and(crate::OutputAvailability::has_definition);
    let state = if optimized {
        terminal.positive("llvm body")
    } else if pre_optimization {
        terminal.warning("pre-opt body only")
    } else {
        terminal.warning("no body")
    };

    let mut output = format!(
        "{}  {}  {}\n",
        terminal.identifier(&display_id.text, display_id.unique_prefix_length),
        state,
        terminal.function(&instance.display_name),
    );
    if disambiguate {
        let origin = instance.source.as_ref().map_or_else(
            || instance.definition.clone(),
            |source| {
                format!(
                    "{} at {}:{}",
                    instance.definition, source.path, source.line_start
                )
            },
        );
        writeln!(
            output,
            "  {}  symbol {}",
            terminal.label(&origin),
            terminal.label(&instance.symbol_fingerprint),
        )
        .expect("writing instance text to a String cannot fail");
    }

    output
}

fn duplicate_display_name(result: &FindResult, instance: &InstanceSummary) -> bool {
    result
        .instances
        .iter()
        .filter(|candidate| candidate.display_name == instance.display_name)
        .count()
        > 1
}

fn selection_text(
    result: &FindResult,
    display_capture: &DisplayIdentifier,
    display_instances: &[DisplayIdentifier],
    selected_output: ShowOutput,
    remark_options: &RemarkOptions,
    include_source: bool,
    terminal: &Terminal,
) -> String {
    let failure = if result.instances.is_empty() {
        SelectionFailure::NotFound
    } else {
        SelectionFailure::Ambiguous
    };
    let mut output = match failure {
        SelectionFailure::NotFound => format!(
            "{} {}.\n",
            terminal.warning("No matching instances in"),
            terminal.identifier(&display_capture.text, display_capture.unique_prefix_length),
        ),
        SelectionFailure::Ambiguous => format!(
            "{} {}\n{}\n",
            terminal.warning("Multiple instances match in capture"),
            terminal.identifier(&display_capture.text, display_capture.unique_prefix_length),
            terminal.heading("Run one command"),
        ),
    };
    for (instance, display_id) in result.instances.iter().zip(display_instances) {
        output.push_str(&instance_text(
            instance,
            display_id,
            duplicate_display_name(result, instance),
            terminal,
        ));
        writeln!(
            output,
            "  {}",
            show_command(
                terminal,
                display_id,
                selected_output,
                (selected_output == ShowOutput::Remarks).then_some(remark_options),
                include_source,
            )
        )
        .expect("writing selection text to a String cannot fail");
    }
    if result.truncated {
        writeln!(
            output,
            "{}",
            terminal.warning(
                "More matching instances exist. Use `cargo optic find` with a larger --limit."
            )
        )
        .expect("writing selection text to a String cannot fail");
    }

    output
}

fn show_command(
    terminal: &Terminal,
    instance_id: &DisplayIdentifier,
    output: ShowOutput,
    remark_options: Option<&RemarkOptions>,
    include_source: bool,
) -> String {
    let mut after = String::new();

    if output != ShowOutput::default() {
        write!(after, " --output {output}").expect("writing command text to a String cannot fail");
    }
    if let Some(options) = remark_options {
        if let Some(kind) = options.kind {
            write!(after, " --kind {}", kind.name())
                .expect("writing command text to a String cannot fail");
        }
        if let Some(pass) = &options.pass {
            write!(after, " --pass={}", shell_quoted(pass))
                .expect("writing command text to a String cannot fail");
        }
        if options.limit != RemarkOptions::DEFAULT_LIMIT {
            write!(after, " --limit {}", options.limit)
                .expect("writing command text to a String cannot fail");
        }
    }
    if include_source {
        after.push_str(" --source");
    }

    terminal.command_with_identifier(
        "cargo optic show --instance ",
        &instance_id.text,
        instance_id.unique_prefix_length,
        &after,
    )
}

/// Quotes one argument for Bash, Zsh, and Fish without shell evaluation.
fn shell_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn show_text(
    view: &ShowView,
    display_capture: &DisplayIdentifier,
    display_instance: &DisplayIdentifier,
    terminal: &Terminal,
) -> String {
    let state = if view.bodies.is_empty() {
        terminal.warning(&format!("no standalone {} body", view.output.name()))
    } else {
        terminal.positive("standalone body")
    };
    let mut output = format!(
        "{} {}\n{}{}\n{}{}\n{}{}\n{}{}\n",
        terminal.heading("Function"),
        terminal.function(&view.instance.display_name),
        terminal.label("  Instance  "),
        terminal.identifier(
            &display_instance.text,
            display_instance.unique_prefix_length,
        ),
        terminal.label("  Capture   "),
        terminal.identifier(&display_capture.text, display_capture.unique_prefix_length),
        terminal.label("  Output    "),
        view.output.title(),
        terminal.label("  State     "),
        state,
    );

    if let Some(source) = &view.source {
        let heading = format!("Source  {}:{}", source.path, source.start_line);
        push_code_section(
            &mut output,
            terminal,
            &heading,
            &source.text,
            CodeSyntax::Rust,
        );
    }
    for body in &view.bodies {
        let heading = format!("{}  {}", view.output.title(), body.module);
        push_code_section(
            &mut output,
            terminal,
            &heading,
            &body.text,
            CodeSyntax::Llvm,
        );
    }

    output
}

fn remark_show_text(
    view: &RemarkShowView,
    display_capture: &DisplayIdentifier,
    display_instance: &DisplayIdentifier,
    terminal: &Terminal,
) -> String {
    let state = match view.summary.state {
        RemarkEvidenceState::NotCaptured => terminal.warning("not captured"),
        RemarkEvidenceState::CapturedEmpty => terminal.warning("captured, no records"),
        RemarkEvidenceState::Captured if view.remarks.is_empty() => {
            terminal.warning("captured; no matching remarks")
        }
        RemarkEvidenceState::Captured => terminal.positive("captured"),
    };
    let mut output = format!(
        "{} {}\n{}{}\n{}{}\n{}{}\n{}{}\n",
        terminal.heading("Function"),
        terminal.function(&view.instance.display_name),
        terminal.label("  Instance  "),
        terminal.identifier(
            &display_instance.text,
            display_instance.unique_prefix_length,
        ),
        terminal.label("  Capture   "),
        terminal.identifier(&display_capture.text, display_capture.unique_prefix_length),
        terminal.label("  Output    "),
        "Optimization remarks",
        terminal.label("  State     "),
        state,
    );

    if let Some(source) = &view.source {
        let heading = format!("Source  {}:{}", source.path, source.start_line);
        push_code_section(
            &mut output,
            terminal,
            &heading,
            &source.text,
            CodeSyntax::Rust,
        );
    }
    if view.summary.state == RemarkEvidenceState::NotCaptured {
        output.push_str(
            "\nCapture remarks with a build-based `cargo optic show QUERY --output remarks` \
             or `cargo optic capture --remarks`.\n",
        );
        return output;
    }

    for remark in &view.remarks {
        let location = remark
            .source_location
            .as_ref()
            .map_or_else(String::new, |location| {
                format!("  {}:{}:{}", location.file, location.line, location.column)
            });
        let hotness = remark
            .hotness
            .map_or_else(String::new, |hotness| format!("  hotness {hotness}"));
        writeln!(
            output,
            "\n{}  {}/{}{}{}\n  {}",
            remark_kind_text(&remark.kind),
            remark.pass_name,
            remark.remark_name,
            location,
            hotness,
            remark.message,
        )
        .expect("writing remark text to a String cannot fail");
    }
    if view.truncated {
        writeln!(
            output,
            "{}",
            terminal.warning("More remarks match. Increase --limit (maximum 1000).")
        )
        .expect("writing remark text to a String cannot fail");
    }

    output
}

fn remark_kind_text(kind: &crate::RemarkKind) -> &str {
    match kind {
        crate::RemarkKind::Passed => "passed",
        crate::RemarkKind::Missed => "missed",
        crate::RemarkKind::Analysis => "analysis",
        crate::RemarkKind::AnalysisFpCommute => "analysis-fp-commute",
        crate::RemarkKind::AnalysisAliasing => "analysis-aliasing",
        crate::RemarkKind::Failure => "failure",
        crate::RemarkKind::Unknown { .. } => "unknown",
    }
}

fn push_code_section(
    output: &mut String,
    terminal: &Terminal,
    heading: &str,
    code: &str,
    syntax: CodeSyntax,
) {
    output.push('\n');
    output.push_str(&terminal.heading(heading));
    output.push_str("\n\n");
    output.push_str(&terminal.code(code, syntax));

    if !code.ends_with('\n') {
        output.push('\n');
    }
}

fn normalized_arguments() -> Vec<OsString> {
    let mut arguments: Vec<_> = env::args_os().collect();

    if arguments.get(1).is_some_and(|argument| argument == "optic") {
        arguments.remove(1);
    }

    arguments
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{Cli, Command, DisplayIdentifier, ShowOutput, find_text, show_command};
    use crate::{
        CaptureId, FindMatchKind, FindResult, InstanceId, InstanceSummary, RemarkKindFilter,
        RemarkOptions, SourceLocation,
    };

    #[test]
    fn displays_twelve_hexadecimal_characters() {
        let identifier = DisplayIdentifier::new("cap_0123456789abcdef0123456789abcdef", "cap_0");

        assert_eq!(identifier.text, "cap_0123456789ab");
        assert_eq!(identifier.unique_prefix_length, 5);
    }

    #[test]
    fn expands_to_include_a_long_unique_prefix() {
        let identifier =
            DisplayIdentifier::new("ins_0123456789abcdef0123456789abcdef", "ins_0123456789abc");

        assert_eq!(identifier.text, "ins_0123456789abc");
        assert_eq!(identifier.unique_prefix_length, 17);
    }

    #[test]
    fn parses_remark_options_without_widening_compare() {
        let capture = Cli::try_parse_from(["cargo optic", "capture", "--remarks"])
            .expect("capture accepts the remark policy");
        assert!(matches!(
            capture.command,
            Command::Capture { build, .. } if build.remarks
        ));

        let show = Cli::try_parse_from([
            "cargo optic",
            "show",
            "--instance",
            "ins_0",
            "--output",
            "remarks",
            "--kind",
            "missed",
            "--pass",
            "loop-vectorize",
            "--limit",
            "17",
        ])
        .expect("show accepts remark filters");
        assert!(matches!(
            show.command,
            Command::Show {
                output: ShowOutput::Remarks,
                kind: Some(RemarkKindFilter::Missed),
                pass_name: Some(ref pass),
                limit: Some(17),
                ..
            } if pass == "loop-vectorize"
        ));

        assert!(
            Cli::try_parse_from([
                "cargo optic",
                "compare",
                "--before",
                "ins_0",
                "--after",
                "ins_1",
                "--output",
                "remarks",
            ])
            .is_err()
        );
    }

    #[test]
    fn quotes_pass_names_in_generated_show_commands() {
        let instance = DisplayIdentifier::full("ins_0123456789abcdef0123456789abcdef");
        let options = RemarkOptions {
            pass: Some("foo bar; $(echo unsafe) 'quoted'".to_owned()),
            ..RemarkOptions::default()
        };

        let command = show_command(
            &crate::terminal::Terminal::new(false),
            &instance,
            ShowOutput::Remarks,
            Some(&options),
            false,
        );

        assert_eq!(
            command,
            concat!(
                "cargo optic show --instance ins_0123456789abcdef0123456789abcdef ",
                "--output remarks --pass='foo bar; $(echo unsafe) '",
                "\"'\"'quoted'\"'\"''",
            )
        );
    }

    #[test]
    fn plain_find_disambiguates_only_duplicate_display_names() {
        let duplicate = instance_summary(
            "ins_11111111111111111111111111111111",
            "same",
            "crate_a::first",
            "_Rfirst",
            "111111111111",
            Some("src/lib.rs"),
        );
        let second_duplicate = instance_summary(
            "ins_22222222222222222222222222222222",
            "same",
            "crate_a::second",
            "_Rsecond",
            "222222222222",
            None,
        );
        let unique = instance_summary(
            "ins_33333333333333333333333333333333",
            "unique",
            "crate_a::unique",
            "_Runique",
            "abcdefabcdef",
            None,
        );
        let result = FindResult {
            capture_id: "cap_11111111111111111111111111111111"
                .parse::<CaptureId>()
                .expect("the capture ID is valid"),
            match_kind: FindMatchKind::Substring,
            truncated: false,
            instances: vec![duplicate, second_duplicate, unique],
        };
        let display_capture = DisplayIdentifier::full(result.capture_id.as_str());
        let display_instances = result
            .instances
            .iter()
            .map(|instance| DisplayIdentifier::full(instance.id.as_str()))
            .collect::<Vec<_>>();

        let output = find_text(
            &result,
            &display_capture,
            &display_instances,
            &crate::terminal::Terminal::new(false),
        );

        assert!(output.contains("crate_a::first at src/lib.rs:7  symbol 111111111111"));
        assert!(output.contains("crate_a::second  symbol 222222222222"));
        assert!(!output.contains("symbol abcdefabcdef"));
        assert!(!output.contains("_Rfirst"));
    }

    #[cfg(unix)]
    #[test]
    fn reports_non_utf8_json_paths_as_errors() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::path::PathBuf;

        use super::{Format, success};
        use crate::CleanSummary;

        let summary = CleanSummary {
            path: PathBuf::from(OsString::from_vec(vec![0xff])),
            removed: true,
        };

        assert!(success(Format::Json, &summary, String::new()).is_err());
    }

    fn instance_summary(
        id: &str,
        display_name: &str,
        definition: &str,
        compiler_symbol: &str,
        symbol_fingerprint: &str,
        source_path: Option<&str>,
    ) -> InstanceSummary {
        InstanceSummary {
            id: id.parse::<InstanceId>().expect("the instance ID is valid"),
            crate_name: "crate_a".to_owned(),
            definition: definition.to_owned(),
            display_name: display_name.to_owned(),
            compiler_symbol: compiler_symbol.to_owned(),
            symbol_fingerprint: symbol_fingerprint.to_owned(),
            source: source_path.map(|path| SourceLocation {
                path: path.to_owned(),
                byte_start: 0,
                byte_end: 1,
                line_start: 7,
                column_start: 0,
                line_end: 7,
                column_end: 1,
            }),
            availability: Vec::new(),
        }
    }
}
