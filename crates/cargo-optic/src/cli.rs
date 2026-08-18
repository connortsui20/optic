//! Implements the human and agent command-line interface.
//!
//! Plain text is the default transport for source and LLVM bodies. `--format json` wraps the same
//! typed application views in a versioned envelope. Read-only commands always require explicit
//! capture IDs and never mutate shared navigation state.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use cargo_ir::CargoTarget;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::{
    Application, BuildSpec, CachePolicy, CaptureId, CaptureSummary, FindResult, InstanceId,
    InstanceSummary, ShowView,
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
    version,
    about = "Inspect LLVM output from real Cargo builds"
)]
struct Cli {
    /// Uses the specified Cargo manifest.
    #[arg(long, global = true, value_name = "PATH")]
    manifest_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Captures enriched compiler evidence for one Cargo target.
    Capture {
        /// The Cargo build and cache options.
        #[command(flatten)]
        build: BuildOptions,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Lists completed captures.
    Captures {
        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Finds concrete instances in one capture.
    Find {
        /// Selects an immutable completed capture.
        #[arg(long)]
        capture: CaptureId,

        /// Matches a definition or concrete compiler instance.
        query: String,

        /// Selects plain text or versioned JSON output.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Shows LLVM bodies for one concrete instance.
    Show {
        /// Matches a definition or concrete compiler instance.
        query: Option<String>,

        /// Selects an immutable capture without starting a build.
        #[arg(long)]
        capture: Option<CaptureId>,

        /// Selects one exact instance from the capture.
        #[arg(long)]
        instance: Option<InstanceId>,

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

struct Execution {
    code: u8,
    output: String,
}

struct Failure {
    format: Format,
    message: String,
}

fn execute(application: &mut Application, cli: Cli) -> Result<Execution, Failure> {
    let manifest_path = cli.manifest_path;

    match cli.command {
        Command::Capture { build, format } => {
            eprintln!("Capturing enriched compiler evidence...");
            let spec = build.to_spec(manifest_path);
            let summary = application
                .capture(&spec, build.cache_policy())
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;

            Ok(success(format, &summary, capture_text(&summary)))
        }
        Command::Captures { format } => {
            let captures = application.captures().map_err(|error| Failure {
                format,
                message: error.to_string(),
            })?;

            Ok(success(format, &captures, captures_text(&captures)))
        }
        Command::Find {
            capture,
            query,
            format,
        } => {
            let result = application
                .find(&capture, &query)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;

            Ok(success(format, &result, find_text(&result)))
        }
        Command::Show {
            query,
            capture,
            instance,
            source,
            build,
            format,
        } => {
            let capture = match capture {
                Some(capture) => {
                    if build.has_build_selection() {
                        return Err(Failure {
                            format,
                            message: "--capture cannot be combined with Cargo build options"
                                .to_owned(),
                        });
                    }
                    if instance.is_some() && query.is_some() {
                        return Err(Failure {
                            format,
                            message: "--instance cannot be combined with a query".to_owned(),
                        });
                    }

                    capture
                }
                None => {
                    if instance.is_some() {
                        return Err(Failure {
                            format,
                            message: "--instance requires --capture".to_owned(),
                        });
                    }
                    if query.is_none() {
                        return Err(Failure {
                            format,
                            message: "show requires a query when it captures a build".to_owned(),
                        });
                    }
                    eprintln!("Capturing enriched compiler evidence...");
                    application
                        .capture(&build.to_spec(manifest_path), build.cache_policy())
                        .map_err(|error| Failure {
                            format,
                            message: error.to_string(),
                        })?
                        .id
                }
            };
            let instance = match instance {
                Some(instance) => instance,
                None => {
                    let Some(query) = query else {
                        return Err(Failure {
                            format,
                            message: "show requires a query or --instance".to_owned(),
                        });
                    };
                    let result = application
                        .find(&capture, &query)
                        .map_err(|error| Failure {
                            format,
                            message: error.to_string(),
                        })?;

                    return select_and_show(application, result, source, format);
                }
            };
            let view = application
                .show(&capture, &instance, source)
                .map_err(|error| Failure {
                    format,
                    message: error.to_string(),
                })?;

            Ok(success(format, &view, show_text(&view)))
        }
    }
}

fn select_and_show(
    application: &Application,
    result: FindResult,
    include_source: bool,
    format: Format,
) -> Result<Execution, Failure> {
    if result.instances.len() != 1 {
        let code = if result.instances.is_empty() {
            "not_found"
        } else {
            "ambiguous"
        };
        let text = selection_text(&result, code);

        return Ok(selection(format, &result, code, text));
    }

    let instance = &result.instances[0];
    let view = application
        .show(&result.capture_id, &instance.id, include_source)
        .map_err(|error| Failure {
            format,
            message: error.to_string(),
        })?;

    Ok(success(format, &view, show_text(&view)))
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
                    "message": "the query did not select exactly one compiler instance",
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

fn capture_text(summary: &CaptureSummary) -> String {
    format!(
        "capture  {}\nstatus   {}\nrustc    {}\nLLVM     {}\ntarget   {}\ninstances {}\n",
        summary.id,
        if summary.reused { "reused" } else { "captured" },
        summary.rustc_release,
        summary.llvm_version,
        summary.target,
        summary.instance_count,
    )
}

fn captures_text(captures: &[CaptureSummary]) -> String {
    if captures.is_empty() {
        return "No captures.\n".to_owned();
    }

    let mut output = String::new();
    for capture in captures {
        output.push_str(&format!(
            "{}  {}  {}  {} instances\n",
            capture.id, capture.rustc_release, capture.target, capture.instance_count
        ));
    }

    output
}

fn find_text(result: &FindResult) -> String {
    if result.instances.is_empty() {
        return format!("No matching instances in {}.\n", result.capture_id);
    }

    let mut output = format!("capture {}\n", result.capture_id);
    for instance in &result.instances {
        output.push_str(&instance_text(instance));
    }

    output
}

fn instance_text(instance: &InstanceSummary) -> String {
    format!(
        "{}  {}  {}\n",
        instance.id,
        if instance.has_body { "body" } else { "no-body" },
        instance.display_name,
    )
}

fn selection_text(result: &FindResult, code: &str) -> String {
    let mut output = if code == "not_found" {
        format!("No matching instances in {}.\n", result.capture_id)
    } else {
        format!(
            "The query is ambiguous in {}. Select one instance:\n",
            result.capture_id
        )
    };
    for instance in &result.instances {
        output.push_str(&instance_text(instance));
    }

    output
}

fn show_text(view: &ShowView) -> String {
    let mut output = format!(
        "capture  {}\ninstance {}\nfunction {}\nstate    {}\n",
        view.capture_id,
        view.instance.id,
        view.instance.display_name,
        if view.instance.has_body {
            "standalone body"
        } else {
            "collected without a supported standalone body"
        },
    );

    if let Some(source) = &view.source {
        output.push_str(&format!(
            "\n===== source: {}:{} =====\n{}",
            source.path, source.start_line, source.text
        ));
    }
    for body in &view.bodies {
        output.push_str(&format!(
            "\n===== {}: {} =====\n{}",
            body.stage, body.module, body.text
        ));
    }

    output
}

fn normalized_arguments() -> Vec<OsString> {
    let mut arguments: Vec<_> = env::args_os().collect();

    if arguments.get(1).is_some_and(|argument| argument == "optic") {
        arguments.remove(1);
    }

    arguments
}
