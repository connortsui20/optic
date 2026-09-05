//! Resolves and executes one validated build request.
//!
//! [`run_build`] binds a [`BuildRequest`] to exact Cargo metadata and invokes `cargo rustc` from
//! the original invocation directory. A successful return records the invocation. It does not
//! claim that Cargo invoked rustc because Cargo can reuse a fresh target.
//!
//! Cargo Optic uses `cargo rustc` instead of `cargo build` because capture must support additional
//! code-generation output. This command keeps Cargo's dependency planning and lets later capture
//! stages pass compiler flags only to the final invocation for the selected target.

use std::process::Command;
use std::process::Stdio;

use cargo_metadata::Metadata;
use cargo_metadata::Package;
use cargo_metadata::Target;
use cargo_metadata::TargetKind;
use optic_records::BuildRecord;
use optic_records::TargetRecord;
use snafu::ResultExt;

use crate::BuildRequest;
use crate::CargoTarget;
use crate::Error;
use crate::Workspace;
use crate::error::PackageNotFoundSnafu;
use crate::error::ProcessFailedSnafu;
use crate::error::StartProcessSnafu;
use crate::error::TargetNotFoundSnafu;

/// Runs one explicit Cargo target and returns its resolved build provenance.
///
/// Cargo inherits the current process's standard input, output, and error streams.
///
/// # Errors
///
/// Returns an error if the request does not resolve to one workspace target or `cargo rustc` fails.
pub fn run_build(workspace: &Workspace, request: &BuildRequest) -> Result<BuildRecord, Error> {
    let metadata = workspace.read_metadata(request)?;
    let package = resolve_package(&metadata, request.package())?;
    let target = resolve_target(package, request.target())?;
    let arguments = cargo_arguments(request);

    let status = Command::new(workspace.cargo())
        .current_dir(workspace.invocation_directory())
        .args(&arguments)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|_| StartProcessSnafu {
            program: workspace.cargo().to_owned(),
        })?;
    if !status.success() {
        return ProcessFailedSnafu {
            program: workspace.cargo().to_owned(),
            status: status.to_string(),
        }
        .fail();
    }

    let target = TargetRecord::new(target.name.clone(), request.target().kind())?;

    let record = BuildRecord::new(
        package.name.to_string(),
        package.version.to_string(),
        target,
        request.profile(),
        workspace.cargo().to_owned(),
        workspace.invocation_directory().to_owned(),
        arguments,
    )?;

    Ok(record)
}

fn resolve_package<'a>(metadata: &'a Metadata, name: &str) -> Result<&'a Package, Error> {
    let package = metadata
        .workspace_packages()
        .into_iter()
        .find(|package| package.name == name);
    let Some(package) = package else {
        return PackageNotFoundSnafu { package: name }.fail();
    };

    Ok(package)
}

fn resolve_target<'a>(package: &'a Package, selected: &CargoTarget) -> Result<&'a Target, Error> {
    let target = package.targets.iter().find(|target| match selected {
        CargoTarget::Library => target.kind.iter().any(|kind| {
            matches!(
                kind,
                TargetKind::Lib
                    | TargetKind::RLib
                    | TargetKind::DyLib
                    | TargetKind::CDyLib
                    | TargetKind::StaticLib
                    | TargetKind::ProcMacro
            )
        }),
        CargoTarget::Binary(name) => target.is_bin() && target.name == *name,
        CargoTarget::Example(name) => target.is_example() && target.name == *name,
        CargoTarget::Benchmark(name) => target.is_bench() && target.name == *name,
    });

    let Some(target) = target else {
        return TargetNotFoundSnafu {
            package: package.name.to_string(),
            target: selected.to_string(),
        }
        .fail();
    };

    Ok(target)
}

fn cargo_arguments(request: &BuildRequest) -> Vec<String> {
    let mut arguments = vec![
        "rustc".to_owned(),
        "--package".to_owned(),
        request.package().to_owned(),
    ];

    arguments.extend(request.target().selector_arguments());
    arguments.push("--profile".to_owned());
    arguments.push(request.profile().to_owned());

    if !request.features().is_empty() {
        arguments.push("--features".to_owned());
        arguments.push(request.features().join(","));
    }
    if request.all_features() {
        arguments.push("--all-features".to_owned());
    }
    if request.no_default_features() {
        arguments.push("--no-default-features".to_owned());
    }

    arguments
}

#[cfg(test)]
mod tests {
    use super::cargo_arguments;
    use crate::BuildRequest;
    use crate::CargoTarget;

    #[test]
    fn maps_each_target_to_exact_cargo_arguments() {
        let cases = [
            (CargoTarget::Library, vec!["--lib"]), // Library target.
            (
                CargoTarget::Binary("tool".to_owned()),
                vec!["--bin", "tool"],
            ), // Binary target.
            (
                CargoTarget::Example("demo".to_owned()),
                vec!["--example", "demo"],
            ), // Example target.
            (
                CargoTarget::Benchmark("scan".to_owned()),
                vec!["--bench", "scan"],
            ), // Benchmark target.
        ];

        for (target, expected_target_arguments) in cases {
            let request = BuildRequest::new("example", target, "release")
                .expect("the fixture request is valid");
            let arguments = cargo_arguments(&request);
            let target_arguments = &arguments[3..arguments.len() - 2];

            assert_eq!(&arguments[..3], ["rustc", "--package", "example"]);
            assert_eq!(target_arguments, expected_target_arguments);
            assert_eq!(&arguments[arguments.len() - 2..], ["--profile", "release"]);
        }
    }

    #[test]
    fn forwards_exact_cargo_feature_selection() {
        let request = BuildRequest::new("example", CargoTarget::Library, "release")
            .expect("the fixture request is valid")
            .with_features(vec!["logging".to_owned(), "serde".to_owned()])
            .expect("the fixture features are valid")
            .with_all_features()
            .without_default_features();

        assert_eq!(
            cargo_arguments(&request),
            [
                "rustc",
                "--package",
                "example",
                "--lib",
                "--profile",
                "release",
                "--features",
                "logging,serde",
                "--all-features",
                "--no-default-features",
            ]
        );
    }
}
