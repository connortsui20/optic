//! Parses the `cargo_optic` grammar into valid product commands.
//!
//! Capture syntax requires an exact package, one target subcommand, and one profile selector. The
//! target enum makes zero or multiple target selections unrepresentable after Clap succeeds.
//! [`parse`] then constructs a [`BuildRequest`], establishing the remaining non-empty-name
//! invariants before command dispatch begins.

use clap::ArgGroup;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use optic::BuildRequest;
use optic::CargoTarget;
use optic::InvalidBuildRequest;

#[derive(Debug, Parser)]
#[command(
    name = "cargo_optic",
    bin_name = "cargo_optic",
    version,
    about = "Capture and inspect Cargo build metadata"
)]
struct Cli {
    #[command(subcommand)]
    command: ParsedCommand,
}

#[derive(Debug, Subcommand)]
enum ParsedCommand {
    /// Runs one explicit Cargo target and remembers the completed build.
    Capture(CaptureOptions),

    /// Lists completed captures from newest to oldest.
    Captures,
}

#[derive(Debug, Args)]
#[command(subcommand_value_name = "TARGET", subcommand_help_heading = "Targets")]
#[command(group(
    ArgGroup::new("profile-selection")
        .required(true)
        .multiple(false)
        .args(["release", "profile"])
))]
struct CaptureOptions {
    /// Selects one exact workspace package name.
    #[arg(short = 'p', long = "package")]
    package: String,

    /// Selects Cargo's release profile.
    #[arg(long)]
    release: bool,

    /// Selects one explicit Cargo profile name.
    #[arg(long)]
    profile: Option<String>,

    #[command(subcommand)]
    target: Target,
}

#[derive(Debug, Subcommand)]
enum Target {
    /// Selects the package's library-like target.
    Lib,

    /// Selects a binary by its Cargo metadata name.
    Bin { name: String },

    /// Selects an example by its Cargo metadata name.
    Example { name: String },

    /// Selects a benchmark by its Cargo metadata name.
    Bench { name: String },
}

pub(crate) enum Command {
    Capture(BuildRequest),

    Captures,
}

pub(crate) fn parse() -> Result<Command, InvalidBuildRequest> {
    let cli = Cli::parse();

    match cli.command {
        ParsedCommand::Capture(options) => Ok(Command::Capture(options.request()?)),
        ParsedCommand::Captures => Ok(Command::Captures),
    }
}

impl CaptureOptions {
    fn request(self) -> Result<BuildRequest, InvalidBuildRequest> {
        let profile = if self.release {
            "release".to_owned()
        } else {
            self.profile
                .expect("clap requires one profile selector before request construction")
        };

        BuildRequest::new(self.package, self.target.into(), profile)
    }
}

impl From<Target> for CargoTarget {
    fn from(target: Target) -> Self {
        match target {
            Target::Lib => Self::Library,
            Target::Bin { name } => Self::Binary(name),
            Target::Example { name } => Self::Example(name),
            Target::Bench { name } => Self::Benchmark(name),
        }
    }
}
