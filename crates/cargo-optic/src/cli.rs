//! Implements the human and agent command-line interface.
//!
//! Plain text is the default transport for source and LLVM bodies. `--format json` wraps the same
//! typed application views in a versioned envelope. Read-only commands use explicit capture or
//! instance IDs and never mutate shared navigation state.

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use cargo_ir::CargoTarget;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::terminal::{CodeSyntax, Terminal};
use crate::{
    Application, BuildSpec, CachePolicy, CaptureId, CaptureSummary, CompilerOutput, FindResult,
    InstanceId, InstanceSummary, ShowView,
};

/// Runs the Cargo Optic CLI and returns its process exit code.
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
            eprintln!("error: failed to read the current directory: {error}");

            return ExitCode::FAILURE;
        }
    };
    if let Some(path) = &mut cli.manifest_path
        && path.is_relative()
    {
        *path = directory.join(&*path);
    }
    let mut application = match Application::discover(&directory, cli.manifest_path.as_deref()) {
        Ok(application) => application,
        Err(error) => return print_error(cli.command.format(), &error.to_string()),
    };

    match execute(&mut application, cli) {
        Ok(execution) => {
            print!("{}", execution.output);
            ExitCode::from(execution.code)
        }
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
    fn format(&self) -> Format {
        match self {
            Self::Capture { format, .. }
            | Self::Captures { format }
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
    fn cache_policy(&self) -> CachePolicy {
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

struct Failure {
    format: Format,
    message: String,
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

fn execute(application: &mut Application, cli: Cli) -> Result<Execution, Failure> {
    let manifest_path = cli.manifest_path;
    let color = cli.color;

    match cli.command {
        Command::Capture { build, format } => {
            let terminal = Terminal::new(color.enabled(format));
            eprintln!("Resolving compiler evidence...");
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

            Ok(success(
                format,
                &summary,
                capture_text(&summary, &display_id, &terminal),
            ))
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

            Ok(success(
                format,
                &captures,
                captures_text(&captures, &display_ids, &terminal),
            ))
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
            let display_instances =
                unique_instance_prefixes(application, &result.instances, format).map_err(
                    |error| Failure {
                        format,
                        message: error.to_string(),
                    },
                )?;

            Ok(success(
                format,
                &result,
                find_text(&result, &display_capture, &display_instances, &terminal),
            ))
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

        return Ok(success(
            format,
            &view,
            show_text(&view, &display_capture, &display_instance, &terminal),
        ));
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
        eprintln!("Resolving compiler evidence...");
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
        result,
        output,
        include_source,
        format,
        &terminal,
    )
}

fn select_and_show(
    application: &Application,
    result: FindResult,
    output: CompilerOutput,
    include_source: bool,
    format: Format,
    terminal: &Terminal,
) -> Result<Execution, Failure> {
    if result.instances.len() != 1 {
        let code = if result.instances.is_empty() {
            "not_found"
        } else {
            "ambiguous"
        };
        let display_capture =
            display_capture_id(application, &result.capture_id, format).map_err(|error| {
                Failure {
                    format,
                    message: error.to_string(),
                }
            })?;
        let display_instances = unique_instance_prefixes(application, &result.instances, format)
            .map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;
        let text = selection_text(
            &result,
            code,
            &display_capture,
            &display_instances,
            output,
            include_source,
            terminal,
        );

        return Ok(selection(format, &result, code, text));
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

    Ok(success(
        format,
        &view,
        show_text(&view, &display_capture, &display_instance, terminal),
    ))
}

fn unique_instance_prefixes(
    application: &Application,
    instances: &[InstanceSummary],
    format: Format,
) -> crate::Result<Vec<InstanceId>> {
    instances
        .iter()
        .map(|instance| display_instance_id(application, &instance.id, format))
        .collect()
}

fn display_capture_id(
    application: &Application,
    capture_id: &CaptureId,
    format: Format,
) -> crate::Result<CaptureId> {
    match format {
        Format::Text => application.unique_capture_prefix(capture_id),
        Format::Json => Ok(capture_id.clone()),
    }
}

fn display_instance_id(
    application: &Application,
    instance_id: &InstanceId,
    format: Format,
) -> crate::Result<InstanceId> {
    match format {
        Format::Text => application.unique_instance_prefix(instance_id),
        Format::Json => Ok(instance_id.clone()),
    }
}

fn success<T: Serialize>(format: Format, result: &T, text: String) -> Execution {
    let output = match format {
        Format::Text => text,
        Format::Json => {
            serde_json::to_string_pretty(&serde_json::json!({
                "version": 1,
                "ok": true,
                "result": result,
            }))
            .expect("serializable application views produce JSON")
                + "\n"
        }
    };

    Execution { code: 0, output }
}

fn selection<T: Serialize>(format: Format, result: &T, code: &str, text: String) -> Execution {
    let output = match format {
        Format::Text => text,
        Format::Json => {
            serde_json::to_string_pretty(&serde_json::json!({
                "version": 1,
                "ok": false,
                "error": {
                    "code": code,
                    "message": "the query must match exactly one compiler instance",
                    "result": result,
                },
            }))
            .expect("serializable selection results produce JSON")
                + "\n"
        }
    };

    Execution { code: 2, output }
}

fn print_error(format: Format, message: &str) -> ExitCode {
    match format {
        Format::Text => eprintln!("error: {message}"),
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "version": 1,
                "ok": false,
                "error": {
                    "code": "operation_failed",
                    "message": message,
                },
            }))
            .expect("static error envelopes produce JSON")
        ),
    }

    ExitCode::FAILURE
}

fn capture_text(summary: &CaptureSummary, display_id: &CaptureId, terminal: &Terminal) -> String {
    let status = if summary.reused { "reused" } else { "captured" };
    let find = format!("cargo optic find --capture {display_id} QUERY");
    let show = format!("cargo optic show QUERY --capture {display_id}");

    format!(
        "{} {}\n{}{}\n{}{}\n{}{}\n{}{}\n\n{}\n  {}\n  {}\n",
        terminal.heading("Capture"),
        terminal.identifier(&display_id.to_string()),
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
        terminal.command(&find),
        terminal.command(&show),
    )
}

fn captures_text(
    captures: &[CaptureSummary],
    display_ids: &[CaptureId],
    terminal: &Terminal,
) -> String {
    if captures.is_empty() {
        return format!("{}\n", terminal.warning("No captures."));
    }

    let mut output = format!("{}\n", terminal.heading("Captures"));
    for (capture, display_id) in captures.iter().zip(display_ids) {
        output.push_str(&format!(
            "{}  {}  {}  {} instances\n",
            terminal.identifier(&display_id.to_string()),
            capture.rustc_release,
            capture.target,
            capture.instance_count,
        ));
    }

    output
}

fn find_text(
    result: &FindResult,
    display_capture: &CaptureId,
    display_instances: &[InstanceId],
    terminal: &Terminal,
) -> String {
    if result.instances.is_empty() {
        return format!(
            "{} {}.\n",
            terminal.warning("No matching instances in"),
            terminal.identifier(&display_capture.to_string()),
        );
    }

    let mut output = format!(
        "{} {}\n",
        terminal.heading("Capture"),
        terminal.identifier(&display_capture.to_string()),
    );
    for (instance, display_id) in result.instances.iter().zip(display_instances) {
        output.push_str(&instance_text(instance, display_id, terminal));
        output.push_str(&format!(
            "  {}\n",
            terminal.command(&show_command(display_id, CompilerOutput::default(), false))
        ));
    }

    output
}

fn instance_text(
    instance: &InstanceSummary,
    display_id: &InstanceId,
    terminal: &Terminal,
) -> String {
    let state = if instance.has_body {
        terminal.positive("body")
    } else {
        terminal.warning("no body")
    };

    format!(
        "{}  {}  {}\n",
        terminal.identifier(&display_id.to_string()),
        state,
        terminal.function(&instance.display_name),
    )
}

fn selection_text(
    result: &FindResult,
    code: &str,
    display_capture: &CaptureId,
    display_instances: &[InstanceId],
    compiler_output: CompilerOutput,
    include_source: bool,
    terminal: &Terminal,
) -> String {
    let mut output = if code == "not_found" {
        format!(
            "{} {}.\n",
            terminal.warning("No matching instances in"),
            terminal.identifier(&display_capture.to_string()),
        )
    } else {
        format!(
            "{} {}\n{}\n",
            terminal.warning("Multiple instances match in capture"),
            terminal.identifier(&display_capture.to_string()),
            terminal.heading("Run one command"),
        )
    };
    for (instance, display_id) in result.instances.iter().zip(display_instances) {
        output.push_str(&instance_text(instance, display_id, terminal));
        output.push_str(&format!(
            "  {}\n",
            terminal.command(&show_command(display_id, compiler_output, include_source))
        ));
    }

    output
}

fn show_command(
    instance_id: &InstanceId,
    compiler_output: CompilerOutput,
    include_source: bool,
) -> String {
    let mut command = format!("cargo optic show --instance {instance_id}");

    if compiler_output != CompilerOutput::default() {
        command.push_str(&format!(" --output {compiler_output}"));
    }
    if include_source {
        command.push_str(" --source");
    }

    command
}

fn show_text(
    view: &ShowView,
    display_capture: &CaptureId,
    display_instance: &InstanceId,
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
        terminal.identifier(&display_instance.to_string()),
        terminal.label("  Capture   "),
        terminal.identifier(&display_capture.to_string()),
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
