//! Parses the `cargo optic` grammar into valid product commands.
//!
//! Cargo [custom subcommands] pass `optic` to the `cargo-optic` executable before the user's
//! arguments. [`Cargo`] models that outer invocation so Clap accepts the repeated command name and
//! renders help with the public `cargo optic` spelling.
//!
//! Capture syntax requires an exact package, one Cargo-style target selector, and one profile
//! selector. [`parse`] constructs a [`BuildRequest`] after Clap establishes those selection
//! invariants.
//!
//! [custom subcommands]:
//!     https://doc.rust-lang.org/cargo/reference/external-tools.html#custom-subcommands

use clap::ArgGroup;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use optic::BuildRequest;
use optic::CargoTarget;
use optic::InvalidBuildRequest;

/// The outer command form that Cargo passes to this executable.
#[derive(Debug, Parser)]
#[command(bin_name = "cargo")]
enum Cargo {
    /// Captures and inspects Cargo build metadata.
    #[command(name = "optic", version)]
    Optic {
        /// The Cargo Optic operation that the user selected.
        #[command(subcommand)]
        command: ParsedCommand,
    },
}

/// A parsed public subcommand before product-request validation.
#[derive(Debug, Subcommand)]
enum ParsedCommand {
    /// Runs one explicit Cargo target and remembers the successful invocation.
    Capture(CaptureOptions),

    /// Lists captures by descending recorded completion time, then ascending capture ID.
    #[command(name = "list-captures")]
    ListCaptures,
}

/// Cargo-style selectors for one explicit build request.
#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("profile-selection")
        .required(true)
        .multiple(false)
        .args(["release", "profile"])
))]
#[command(group(
    ArgGroup::new("target-selection")
        .required(true)
        .multiple(false)
        .args(["lib", "bin", "example", "bench"])
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

    /// Selects the package's library-like target.
    #[arg(long)]
    lib: bool,

    /// Selects a binary by its Cargo metadata name.
    #[arg(long, value_name = "NAME")]
    bin: Option<String>,

    /// Selects an example by its Cargo metadata name.
    #[arg(long, value_name = "NAME")]
    example: Option<String>,

    /// Selects a benchmark by its Cargo metadata name.
    #[arg(long, value_name = "NAME")]
    bench: Option<String>,

    /// Enables a comma-separated list of Cargo features.
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    /// Enables all Cargo features.
    #[arg(long)]
    all_features: bool,

    /// Disables default Cargo features.
    #[arg(long)]
    no_default_features: bool,
}

/// A validated operation ready for application dispatch.
pub(crate) enum Command {
    /// Captures one explicit Cargo build request.
    Capture(BuildRequest),

    /// Lists the completed capture history.
    ListCaptures,
}

pub(crate) fn parse() -> Result<Command, InvalidBuildRequest> {
    let Cargo::Optic { command } = Cargo::parse();

    match command {
        ParsedCommand::Capture(options) => Ok(Command::Capture(options.into_request()?)),
        ParsedCommand::ListCaptures => Ok(Command::ListCaptures),
    }
}

impl CaptureOptions {
    fn into_request(self) -> Result<BuildRequest, InvalidBuildRequest> {
        let target = if self.lib {
            CargoTarget::Library
        } else if let Some(name) = self.bin {
            CargoTarget::Binary(name)
        } else if let Some(name) = self.example {
            CargoTarget::Example(name)
        } else if let Some(name) = self.bench {
            CargoTarget::Benchmark(name)
        } else {
            unreachable!("Clap requires one target selector before request construction")
        };

        let profile = if self.release {
            "release".to_owned()
        } else {
            self.profile
                .expect("Clap requires one profile selector before request construction")
        };

        let mut request =
            BuildRequest::new(self.package, target, profile)?.with_features(self.features)?;
        if self.all_features {
            request = request.with_all_features();
        }
        if self.no_default_features {
            request = request.without_default_features();
        }

        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use clap::error::ErrorKind;
    use optic::CargoTarget;

    use super::CaptureOptions;
    use super::Cargo;
    use super::ParsedCommand;

    fn capture_options(arguments: Vec<&str>) -> CaptureOptions {
        let Cargo::Optic { command } =
            Cargo::try_parse_from(arguments).expect("the fixture command is valid");
        let ParsedCommand::Capture(options) = command else {
            panic!("the fixture command is a capture command");
        };

        options
    }

    #[track_caller]
    fn assert_parse_error(arguments: &[&str], expected: ErrorKind) {
        let error = Cargo::try_parse_from(arguments)
            .expect_err("the invalid fixture command must be rejected");

        assert_eq!(error.kind(), expected);
    }

    #[test]
    fn accepts_each_cargo_target_selector() {
        let cases = [
            ("--lib", None, CargoTarget::Library), // Library target.
            (
                "--bin",
                Some("tool"),
                CargoTarget::Binary("tool".to_owned()),
            ), // Binary target.
            (
                "--example",
                Some("demo"),
                CargoTarget::Example("demo".to_owned()),
            ), // Example target.
            (
                "--bench",
                Some("scan"),
                CargoTarget::Benchmark("scan".to_owned()),
            ), // Benchmark target.
        ];

        for (selector, name, expected) in cases {
            let mut arguments = vec![
                "cargo",
                "optic",
                "capture",
                "--package",
                "example",
                "--release",
                selector,
            ];
            arguments.extend(name);
            let options = capture_options(arguments);
            let request = options
                .into_request()
                .expect("the fixture request is valid");

            assert_eq!(request.target(), &expected);
        }
    }

    #[test]
    fn requires_exactly_one_target_selector() {
        let missing_target = [
            "cargo",
            "optic",
            "capture",
            "--package",
            "example",
            "--release",
        ];
        let conflicting_targets = [
            "cargo",
            "optic",
            "capture",
            "--package",
            "example",
            "--release",
            "--lib",
            "--bin",
            "tool",
        ];

        assert_parse_error(&missing_target, ErrorKind::MissingRequiredArgument);
        assert_parse_error(&conflicting_targets, ErrorKind::ArgumentConflict);
    }

    #[test]
    fn requires_exactly_one_profile_selector() {
        let missing_profile = ["cargo", "optic", "capture", "--package", "example", "--lib"];
        let conflicting_profiles = [
            "cargo",
            "optic",
            "capture",
            "--package",
            "example",
            "--lib",
            "--release",
            "--profile",
            "dev",
        ];

        assert_parse_error(&missing_profile, ErrorKind::MissingRequiredArgument);
        assert_parse_error(&conflicting_profiles, ErrorKind::ArgumentConflict);
    }

    #[test]
    fn accepts_an_explicit_profile() {
        let options = capture_options(vec![
            "cargo",
            "optic",
            "capture",
            "--package",
            "example",
            "--lib",
            "--profile",
            "custom",
        ]);
        let request = options
            .into_request()
            .expect("the fixture request is valid");

        assert_eq!(request.profile(), "custom");
    }

    #[test]
    fn accepts_cargo_feature_selection() {
        let options = capture_options(vec![
            "cargo",
            "optic",
            "capture",
            "--package",
            "example",
            "--release",
            "--lib",
            "--features",
            "logging,serde",
            "--all-features",
            "--no-default-features",
        ]);
        let request = options
            .into_request()
            .expect("the fixture request is valid");

        assert_eq!(request.features(), ["logging", "serde"]);
        assert!(request.all_features());
        assert!(request.no_default_features());
    }

    #[test]
    fn exposes_list_captures_as_the_list_command_name() {
        let Cargo::Optic { command } = Cargo::try_parse_from(["cargo", "optic", "list-captures"])
            .expect("the public list-captures command is valid");

        assert!(matches!(command, ParsedCommand::ListCaptures));
        assert_parse_error(
            &["cargo", "optic", "captures"],
            ErrorKind::InvalidSubcommand,
        );
    }
}
