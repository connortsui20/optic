//! Resolves and executes one validated build request.
//!
//! [`run_build`] binds a [`BuildRequest`] to exact Cargo metadata, records the selected compiler,
//! and invokes `cargo rustc`. The returned [`CompletedBuild`] keeps the build and toolchain records
//! associated and read-only so downstream code cannot rewrite their provenance relationship.

use std::process::Command;

use cargo_metadata::Package;
use cargo_metadata::Target;
use cargo_metadata::TargetKind;
use optic_records::BuildRecord;
use optic_records::TargetRecord;
use optic_records::ToolchainRecord;
use snafu::ResultExt;

use crate::BuildRequest;
use crate::CargoTarget;
use crate::Error;
use crate::PackageNotFoundSnafu;
use crate::ProcessFailedSnafu;
use crate::StartProcessSnafu;
use crate::TargetNotFoundSnafu;
use crate::Workspace;
use crate::toolchain;

/// The provenance produced only after Cargo exits successfully.
pub struct CompletedBuild {
    build: BuildRecord,
    toolchain: ToolchainRecord,
}

impl CompletedBuild {
    /// Returns the resolved package, target, profile, and Cargo command.
    #[must_use]
    pub fn build(&self) -> &BuildRecord {
        &self.build
    }

    /// Returns the compiler identity associated with the Cargo command.
    #[must_use]
    pub fn toolchain(&self) -> &ToolchainRecord {
        &self.toolchain
    }
}

/// Runs one explicit Cargo target and returns its resolved build provenance.
///
/// # Errors
///
/// Returns an error if the request does not resolve to one workspace target, the selected rustc
/// cannot be inspected, or `cargo rustc` fails.
pub fn run_build(workspace: &Workspace, request: &BuildRequest) -> Result<CompletedBuild, Error> {
    let package = resolve_package(workspace, request.package())?;
    let target = resolve_target(package, request.target())?;
    let rustc = toolchain::selected_rustc(workspace.cargo(), workspace.root())?;
    let toolchain = toolchain::inspect_rustc(&rustc, workspace.root())?;
    let arguments = cargo_arguments(request);

    let status = Command::new(workspace.cargo())
        .current_dir(workspace.root())
        .args(&arguments)
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
    let build = BuildRecord::new(
        package.name.to_string(),
        package.version.to_string(),
        target,
        request.profile(),
        workspace.cargo().to_owned(),
        arguments,
    )?;

    Ok(CompletedBuild { build, toolchain })
}

fn resolve_package<'a>(workspace: &'a Workspace, name: &str) -> Result<&'a Package, Error> {
    let package = workspace
        .metadata()
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

            assert_eq!(target_arguments, expected_target_arguments);
            assert_eq!(&arguments[arguments.len() - 2..], ["--profile", "release"]);
        }
    }
}
