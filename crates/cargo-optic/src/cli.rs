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

use cargo_ir::CargoTarget;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::terminal::{CodeSyntax, Terminal};
use crate::{
    Application, BuildSpec, CachePolicy, CaptureId, CaptureSummary, CleanSummary, CompilerOutput,
    FindResult, InstanceId, InstanceSummary, ShowView,
};

const MINIMUM_DISPLAY_ID_HEX_DIGITS: usize = 12;

/// Runs the Cargo Optic CLI and returns its process exit code.
#[must_use]
pub fn run_cli() -> ExitCode {
    let arguments = normalized_arguments();
    let mut cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
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

    /// Removes all stored Optic evidence for this workspace.
    #[command(after_long_help = concat!(
        "Example:\n  cargo optic clean\n\n",
        "This command does not remove the Cargo target directory."
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

        /// Selects one compiler output.
        #[arg(long, value_enum, default_value_t)]
        output: CompilerOutput,

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
            | Self::Clean { format }
            | Self::Find { format, .. }
            | Self::Show { format, .. } => *format,
        }
    }
}

#[derive(Clone, Debug, Default, Args)]
struct BuildOptions {
    /// Runs the compiler even if matching evidence exists.
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
            Some(CargoTarget::Library)
        } else if let Some(name) = &self.bin {
            Some(CargoTarget::Binary(name.clone()))
        } else if let Some(name) = &self.bench {
            Some(CargoTarget::Benchmark(name.clone()))
        } else {
            self.example
                .as_ref()
                .map(|name| CargoTarget::Example(name.clone()))
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
        }
    }

    const fn has_build_selection(&self) -> bool {
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
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
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

    output: CompilerOutput,

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
                .capture(&spec, build.cache_policy())
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
        Command::Clean { .. } => {
            unreachable!("clean executes before the application opens its store")
        }
        Command::Find {
            capture,
            query,
            format,
        } => {
            let terminal = Terminal::new(color.enabled(format));
            let result = application
                .find(&capture, &query)
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
            source,
            build,
            format,
        } => execute_show(
            application,
            ShowRequest {
                manifest_path,
                query,
                capture,
                instance,
                output,
                include_source: source,
                build,
                format,
                color,
            },
        ),
    }
}

fn execute_show(application: &mut Application, request: ShowRequest) -> Result<Execution, Failure> {
    let ShowRequest {
        manifest_path,
        query,
        capture,
        instance,
        output,
        include_source,
        build,
        format,
        color,
    } = request;
    let terminal = Terminal::new(color.enabled(format));

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

        let view = application
            .show(&instance, output, include_source)
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
            show_text(&view, &display_capture, &display_instance, &terminal),
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
        application
            .capture(&build.to_spec(manifest_path), build.cache_policy())
            .map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?
            .id
    };
    let result = application
        .find(&capture, &query)
        .map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;

    select_and_show(
        application,
        &result,
        output,
        include_source,
        format,
        &terminal,
    )
}

fn select_and_show(
    application: &Application,
    result: &FindResult,
    output: CompilerOutput,
    include_source: bool,
    format: Format,
    terminal: &Terminal,
) -> Result<Execution, Failure> {
    if result.instances.len() != 1 {
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
            failure,
            &display_capture,
            &display_instances,
            output,
            include_source,
            terminal,
        );

        return selection(format, result, failure, text);
    }

    let instance = &result.instances[0];
    let view = application
        .show(&instance.id, output, include_source)
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
        show_text(&view, &display_capture, &display_instance, terminal),
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
                version: 1,
                ok: true,
                result,
            };

            serde_json::to_string_pretty(&envelope).map_err(|error| Failure {
                format,
                message: format!("failed to encode JSON output: {error}"),
            })? + "\n"
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
                version: 1,
                ok: false,
                error: SelectionEnvelopeError {
                    code: failure.code(),
                    message: "the query must match exactly one compiler instance",
                    result,
                },
            };

            serde_json::to_string_pretty(&envelope).map_err(|error| Failure {
                format,
                message: format!("failed to encode JSON output: {error}"),
            })? + "\n"
        }
    };

    Ok(Execution { code: 2, output })
}

fn print_error(format: Format, message: &str) -> ExitCode {
    let _ = match format {
        Format::Text => write_stderr(&format!("error: {message}\n")),
        Format::Json => {
            let envelope = OperationErrorEnvelope {
                version: 1,
                ok: false,
                error: OperationError {
                    code: "operation_failed",
                    message,
                },
            };
            let output = serde_json::to_string_pretty(&envelope)
                .expect("operation error envelopes contain only strings and integers");

            write_stdout(&format!("{output}\n"))
        }
    };

    ExitCode::FAILURE
}

fn write_progress(message: &str) {
    let _ = write_stderr(&format!("{message}\n"));
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
    let status = if summary.reused { "reused" } else { "captured" };
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
        "{} {}\n{}{}\n{}{}\n{}{}\n{}{}\n\n{}\n  {}\n  {}\n",
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
        terminal.heading("Next commands"),
        find,
        show,
    )
}

fn clean_text(summary: &CleanSummary, terminal: &Terminal) -> String {
    if summary.removed {
        format!(
            "{} at {}.\n",
            terminal.positive("Removed the Optic cache"),
            summary.path.display(),
        )
    } else {
        format!(
            "{} at {}.\n",
            terminal.warning("No Optic cache exists"),
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
            "{}  {}  {}  {} instances",
            terminal.identifier(&display_id.text, display_id.unique_prefix_length),
            capture.rustc_release,
            capture.target,
            capture.instance_count,
        )
        .expect("writing capture text to a String cannot fail");
    }

    output
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
        output.push_str(&instance_text(instance, display_id, terminal));
        writeln!(
            output,
            "  {}",
            show_command(terminal, display_id, CompilerOutput::default(), false)
        )
        .expect("writing instance text to a String cannot fail");
    }

    output
}

fn instance_text(
    instance: &InstanceSummary,
    display_id: &DisplayIdentifier,
    terminal: &Terminal,
) -> String {
    let state = if instance.has_body {
        terminal.positive("body")
    } else {
        terminal.warning("no body")
    };

    format!(
        "{}  {}  {}\n",
        terminal.identifier(&display_id.text, display_id.unique_prefix_length),
        state,
        terminal.function(&instance.display_name),
    )
}

fn selection_text(
    result: &FindResult,
    failure: SelectionFailure,
    display_capture: &DisplayIdentifier,
    display_instances: &[DisplayIdentifier],
    compiler_output: CompilerOutput,
    include_source: bool,
    terminal: &Terminal,
) -> String {
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
        output.push_str(&instance_text(instance, display_id, terminal));
        writeln!(
            output,
            "  {}",
            show_command(terminal, display_id, compiler_output, include_source)
        )
        .expect("writing selection text to a String cannot fail");
    }

    output
}

fn show_command(
    terminal: &Terminal,
    instance_id: &DisplayIdentifier,
    compiler_output: CompilerOutput,
    include_source: bool,
) -> String {
    let mut after = String::new();

    if compiler_output != CompilerOutput::default() {
        write!(after, " --output {compiler_output}")
            .expect("writing command text to a String cannot fail");
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
    use super::DisplayIdentifier;

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
}
