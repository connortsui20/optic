//! Selects the default compiler used for collection.
//!
//! The first implementation uses `rustc` from `PATH`. It rejects compiler overrides and disables
//! configured wrappers instead of reproducing Cargo's complete compiler configuration.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use optic_records::CompilerIdentity;

use crate::Error;
use crate::Workspace;

const RUSTC_ENVIRONMENT: [&str; 2] = ["RUSTC", "CARGO_BUILD_RUSTC"];
const GLOBAL_WRAPPER_ENVIRONMENT: [&str; 2] = ["RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER"];
const WORKSPACE_WRAPPER_ENVIRONMENT: [&str; 2] = [
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
];

/// The default compiler and whether collection must bypass a configured wrapper.
pub(crate) struct CompilerContext {
    identity: CompilerIdentity,
    wrappers_configured: bool,
}

impl CompilerContext {
    /// Inspects `rustc` from `PATH` and rejects explicit compiler selection.
    pub(crate) fn discover(workspace: &Workspace) -> Result<Self, Error> {
        let configuration = read_compiler_configuration(workspace)?;
        if RUSTC_ENVIRONMENT
            .iter()
            .any(|name| env::var_os(name).is_some())
            || configuration.rustc
        {
            return Err(Error::CompilerEnvironment {
                message: "custom rustc selection is not supported; use the default rustc from PATH"
                    .to_owned(),
            });
        }

        let wrappers_configured =
            wrapper_is_configured(&GLOBAL_WRAPPER_ENVIRONMENT, configuration.global_wrapper)
                || wrapper_is_configured(
                    &WORKSPACE_WRAPPER_ENVIRONMENT,
                    configuration.workspace_wrapper,
                );

        Ok(Self {
            identity: inspect_rustc(workspace)?,
            wrappers_configured,
        })
    }

    pub(crate) fn identity(&self) -> &CompilerIdentity {
        &self.identity
    }

    pub(crate) fn wrappers_configured(&self) -> bool {
        self.wrappers_configured
    }
}

#[derive(Default)]
struct CompilerConfiguration {
    rustc: bool,
    global_wrapper: bool,
    workspace_wrapper: bool,
}

fn read_compiler_configuration(workspace: &Workspace) -> Result<CompilerConfiguration, Error> {
    let output = Command::new(workspace.cargo())
        .current_dir(workspace.invocation_directory())
        .args(["-Z", "unstable-options", "config", "get"])
        .env("RUSTC_BOOTSTRAP", "1")
        .output()
        .map_err(|source| Error::StartProcess {
            program: workspace.cargo().to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: workspace.cargo().to_owned(),
            status: output.status.to_string(),
            diagnostics: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }

    let mut configuration = CompilerConfiguration::default();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((field, _)) = line.split_once(" = ") else {
            continue;
        };

        match field {
            "build.rustc" => configuration.rustc = true,
            "build.rustc-wrapper" => configuration.global_wrapper = true,
            "build.rustc-workspace-wrapper" => configuration.workspace_wrapper = true,
            _ => {}
        }
    }

    Ok(configuration)
}

fn wrapper_is_configured(environment: &[&str; 2], configured: bool) -> bool {
    match environment.iter().find_map(env::var_os) {
        Some(value) => !value.is_empty(),
        None => configured,
    }
}

fn inspect_rustc(workspace: &Workspace) -> Result<CompilerIdentity, Error> {
    let verbose = run_rustc(workspace, &["-vV"])?;
    let release = compiler_field(&verbose, "release")?;
    let commit_hash = compiler_field(&verbose, "commit-hash")?;
    let host = compiler_field(&verbose, "host")?;
    let reported_sysroot = PathBuf::from(run_rustc(workspace, &["--print", "sysroot"])?.trim());
    let sysroot = fs::canonicalize(&reported_sysroot).map_err(|source| Error::Filesystem {
        operation: "resolve rustc sysroot",
        path: reported_sysroot,
        source,
    })?;
    let rustc = fs::canonicalize(sysroot.join("bin").join("rustc")).map_err(|source| {
        Error::Filesystem {
            operation: "resolve rustc executable",
            path: sysroot.join("bin").join("rustc"),
            source,
        }
    })?;

    CompilerIdentity::new(rustc, release, commit_hash, host, sysroot).map_err(Error::from)
}

fn run_rustc(workspace: &Workspace, arguments: &[&str]) -> Result<String, Error> {
    let output = Command::new("rustc")
        .current_dir(workspace.invocation_directory())
        .args(arguments)
        .output()
        .map_err(|source| Error::StartProcess {
            program: PathBuf::from("rustc"),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: PathBuf::from("rustc"),
            status: output.status.to_string(),
            diagnostics: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn compiler_field(verbose: &str, name: &'static str) -> Result<String, Error> {
    let value = verbose
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(value) = value else {
        return Err(Error::CompilerEnvironment {
            message: format!("rustc -vV must report {name}, got no value"),
        });
    };

    Ok(value.to_owned())
}
