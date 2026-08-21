//! Discovers Cargo's selected compiler and its matching LLVM tools.
//!
//! Cargo configuration, explicit compiler environment variables, and the active rustup override
//! can all select a compiler. [`CargoContext`] resolves that selection once so configuration
//! discovery, capture, and the exact-version driver use the same Cargo and rustc commands.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const CONFIG_FIELDS: [&str; 3] = [
    "build.rustc",
    "build.rustc-wrapper",
    "build.rustc-workspace-wrapper",
];

/// Cargo and compiler commands selected for one workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CargoContext {
    workspace_root: PathBuf,
    cargo: OsString,
    rustc: PathBuf,
    global_wrapper: Option<OsString>,
    workspace_wrapper: Option<OsString>,
}

impl CargoContext {
    pub(crate) fn discover(workspace_root: &Path) -> Result<Self> {
        let cargo = selected_cargo();
        let configuration = CargoConfiguration::read(&cargo, workspace_root)?;
        let rustc = effective_program(
            "RUSTC",
            configuration.rustc.as_ref(),
            "rustc",
            workspace_root,
        );
        let global_wrapper = effective_wrapper(
            "RUSTC_WRAPPER",
            configuration.global_wrapper.as_ref(),
            workspace_root,
        );
        let workspace_wrapper = effective_wrapper(
            "RUSTC_WORKSPACE_WRAPPER",
            configuration.workspace_wrapper.as_ref(),
            workspace_root,
        );

        Ok(Self {
            workspace_root: workspace_root.to_owned(),
            cargo,
            rustc: PathBuf::from(rustc),
            global_wrapper,
            workspace_wrapper,
        })
    }

    pub(crate) fn cargo(&self) -> &OsStr {
        &self.cargo
    }

    pub(crate) fn rustc(&self) -> &Path {
        &self.rustc
    }

    pub(crate) fn global_wrapper(&self) -> Option<&OsStr> {
        self.global_wrapper.as_deref()
    }

    pub(crate) fn workspace_wrapper(&self) -> Option<&OsStr> {
        self.workspace_wrapper.as_deref()
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

#[derive(Debug, Default)]
struct CargoConfiguration {
    rustc: Option<ConfiguredProgram>,
    global_wrapper: Option<ConfiguredProgram>,
    workspace_wrapper: Option<ConfiguredProgram>,
}

impl CargoConfiguration {
    fn read(cargo: &OsStr, workspace_root: &Path) -> Result<Self> {
        let program = cargo.to_string_lossy().into_owned();
        let output = Command::new(cargo)
            .current_dir(workspace_root)
            .args([
                "-Z",
                "unstable-options",
                "config",
                "get",
                "build",
                "--show-origin",
            ])
            .env("RUSTC_BOOTSTRAP", "1")
            .output()
            .map_err(|source| Error::StartProcess {
                program: program.clone(),
                source,
            })?;
        if !output.status.success() {
            let diagnostics = String::from_utf8_lossy(&output.stderr);
            if diagnostics.contains("config value `build` is not set") {
                return Ok(Self::default());
            }

            return Err(Error::ProcessFailed {
                program,
                status: output.status.to_string(),
                diagnostics: diagnostics.into_owned(),
            });
        }

        Self::parse(&String::from_utf8_lossy(&output.stdout), workspace_root)
    }

    fn parse(output: &str, workspace_root: &Path) -> Result<Self> {
        let mut configuration = Self::default();

        for line in output.lines() {
            let Some((field, remainder)) = line.split_once(" = ") else {
                continue;
            };
            if !CONFIG_FIELDS.contains(&field) {
                continue;
            }
            let Some((encoded_value, origin)) = remainder.rsplit_once(" # ") else {
                return Err(Error::CompilerEnvironment {
                    message: format!("Cargo configuration did not report an origin for {field}"),
                });
            };
            let value = serde_json::from_str::<String>(encoded_value).map_err(|source| {
                Error::CompilerEnvironment {
                    message: format!("Cargo returned an invalid value for {field}: {source}"),
                }
            })?;
            let program = ConfiguredProgram::new(value, origin, workspace_root);

            match field {
                "build.rustc" => configuration.rustc = Some(program),
                "build.rustc-wrapper" => configuration.global_wrapper = Some(program),
                "build.rustc-workspace-wrapper" => {
                    configuration.workspace_wrapper = Some(program);
                }
                _ => unreachable!("CONFIG_FIELDS contains every matched field"),
            }
        }

        Ok(configuration)
    }
}

#[derive(Debug)]
struct ConfiguredProgram {
    value: OsString,
}

impl ConfiguredProgram {
    fn new(value: String, origin: &str, workspace_root: &Path) -> Self {
        let value = resolve_program_path(
            OsString::from(value),
            configuration_base(origin, workspace_root),
        );

        Self { value }
    }
}

/// The exact active compiler and matching LLVM disassembler.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Toolchain {
    /// The compiler executable selected by Cargo.
    pub rustc: PathBuf,

    /// The rustc release string.
    pub release: String,

    /// The rustc commit hash.
    pub commit_hash: String,

    /// The compiler host triple.
    pub host: String,

    /// The embedded LLVM version.
    pub llvm_version: String,

    /// The canonical compiler sysroot.
    pub sysroot: PathBuf,

    /// The directory that contains rustc-private libraries for this host.
    pub rustc_private_lib: PathBuf,

    /// The matching `llvm-dis` executable.
    pub llvm_dis: PathBuf,

    /// The rustup toolchain directory name, when the sysroot proves one.
    pub rustup_toolchain: Option<String>,
}

/// Inspects the compiler selected by Cargo from the current directory.
///
/// # Errors
///
/// Returns an error if Cargo configuration, rustc, or the compiler's matching `llvm-dis` is not
/// available.
pub fn inspect_toolchain() -> Result<Toolchain> {
    let directory = env::current_dir().map_err(|source| Error::Filesystem {
        operation: "read current directory for",
        path: PathBuf::from("."),
        source,
    })?;

    inspect_workspace_toolchain(&directory)
}

/// Inspects the compiler selected by Cargo for `workspace_root`.
///
/// # Errors
///
/// Returns an error if Cargo configuration, rustc, or the compiler's matching `llvm-dis` is not
/// available.
pub fn inspect_workspace_toolchain(workspace_root: &Path) -> Result<Toolchain> {
    let cargo = CargoContext::discover(workspace_root)?;

    inspect_rustc(&cargo)
}

pub(crate) fn inspect_rustc(cargo: &CargoContext) -> Result<Toolchain> {
    let rustc = cargo.rustc();
    let verbose = run_rustc(cargo, ["-vV"])?;
    let release = field(&verbose, "release")?.to_owned();
    let commit_hash = field(&verbose, "commit-hash")?.to_owned();
    let host = field(&verbose, "host")?.to_owned();
    let llvm_version = field(&verbose, "LLVM version")?.to_owned();
    let reported_sysroot = PathBuf::from(run_rustc(cargo, ["--print", "sysroot"])?.trim());
    let sysroot = fs::canonicalize(&reported_sysroot).map_err(|source| Error::Filesystem {
        operation: "canonicalize compiler sysroot",
        path: reported_sysroot,
        source,
    })?;
    let rustc_private_lib = sysroot.join("lib").join("rustlib").join(&host).join("lib");
    let executable = if cfg!(windows) {
        "llvm-dis.exe"
    } else {
        "llvm-dis"
    };
    let llvm_dis = sysroot
        .join("lib")
        .join("rustlib")
        .join(&host)
        .join("bin")
        .join(executable);
    let rustup_toolchain = rustup_toolchain_name(&sysroot);

    if !llvm_dis.is_file() {
        return Err(Error::MissingLlvmDis {
            release: release.clone(),
            commit_hash: commit_hash.clone(),
            path: llvm_dis,
            install_command: rustup_toolchain.as_ref().map(|toolchain| {
                format!("rustup component add --toolchain {toolchain} llvm-tools")
            }),
        });
    }

    Ok(Toolchain {
        rustc: rustc.to_owned(),
        release,
        commit_hash,
        host,
        llvm_version,
        sysroot,
        rustc_private_lib,
        llvm_dis,
        rustup_toolchain,
    })
}

fn selected_cargo() -> OsString {
    env::var_os("CARGO")
        .filter(|cargo| !cargo.is_empty())
        .unwrap_or_else(|| OsString::from("cargo"))
}

fn effective_program(
    environment_name: &str,
    configured: Option<&ConfiguredProgram>,
    default: &str,
    workspace_root: &Path,
) -> OsString {
    effective_program_from(
        env::var_os(environment_name),
        configured,
        default,
        workspace_root,
    )
}

fn effective_program_from(
    environment: Option<OsString>,
    configured: Option<&ConfiguredProgram>,
    default: &str,
    workspace_root: &Path,
) -> OsString {
    environment.map_or_else(
        || {
            configured
                .map(|program| program.value.clone())
                .unwrap_or_else(|| OsString::from(default))
        },
        |value| resolve_program_path(value, workspace_root),
    )
}

fn effective_wrapper(
    environment_name: &str,
    configured: Option<&ConfiguredProgram>,
    workspace_root: &Path,
) -> Option<OsString> {
    effective_wrapper_from(env::var_os(environment_name), configured, workspace_root)
}

fn effective_wrapper_from(
    environment: Option<OsString>,
    configured: Option<&ConfiguredProgram>,
    workspace_root: &Path,
) -> Option<OsString> {
    match environment {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(resolve_program_path(value, workspace_root)),
        None => configured.map(|program| program.value.clone()),
    }
}

fn configuration_base<'a>(origin: &'a str, workspace_root: &'a Path) -> &'a Path {
    if origin.starts_with("environment variable `") || origin.starts_with("command line") {
        return workspace_root;
    }

    Path::new(origin)
        .parent()
        .and_then(Path::parent)
        .unwrap_or(workspace_root)
}

fn resolve_program_path(value: OsString, base: &Path) -> OsString {
    let path = Path::new(&value);
    if path.is_absolute() || !has_path_separator(path) {
        return value;
    }

    base.join(path).into_os_string()
}

fn has_path_separator(path: &Path) -> bool {
    path.components().count() > 1
        || path
            .as_os_str()
            .to_string_lossy()
            .contains(std::path::MAIN_SEPARATOR)
}

fn run_rustc<const N: usize>(cargo: &CargoContext, arguments: [&str; N]) -> Result<String> {
    let rustc = cargo.rustc();
    let program = rustc.to_string_lossy().into_owned();
    let output = Command::new(rustc)
        .current_dir(cargo.workspace_root())
        .args(arguments)
        .output()
        .map_err(|source| Error::StartProcess {
            program: program.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program,
            status: output.status.to_string(),
            diagnostics: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn field<'a>(verbose: &'a str, name: &'static str) -> Result<&'a str> {
    verbose
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(Error::MissingToolchainField { field: name })
}

fn rustup_toolchain_name(sysroot: &Path) -> Option<String> {
    let toolchain_directory = sysroot.parent()?;
    if toolchain_directory.file_name()? != "toolchains" {
        return None;
    }

    sysroot.file_name()?.to_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{
        CargoConfiguration, ConfiguredProgram, configuration_base, effective_program_from,
        effective_wrapper_from, field, resolve_program_path, rustup_toolchain_name,
    };
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    #[test]
    fn parses_cargo_programs_with_their_relative_path_origins() {
        let output = r#"
build.rustc = "tools/rustc" # /workspace/.cargo/config.toml
build.rustc-wrapper = "global-wrapper" # /home/user/.cargo/config.toml
build.rustc-workspace-wrapper = "tools/workspace-wrapper" # environment variable `CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER`
"#;

        let configuration = CargoConfiguration::parse(output, Path::new("/workspace"))
            .expect("the Cargo configuration is valid");

        assert_eq!(
            configuration.rustc.expect("rustc is set").value,
            OsString::from("/workspace/tools/rustc")
        );
        assert_eq!(
            configuration
                .global_wrapper
                .expect("the global wrapper is set")
                .value,
            OsString::from("global-wrapper")
        );
        assert_eq!(
            configuration
                .workspace_wrapper
                .expect("the workspace wrapper is set")
                .value,
            OsString::from("/workspace/tools/workspace-wrapper")
        );
    }

    #[test]
    fn uses_the_workspace_for_environment_and_command_line_paths() {
        assert_eq!(
            configuration_base(
                "environment variable `CARGO_BUILD_RUSTC`",
                Path::new("/workspace")
            ),
            Path::new("/workspace")
        );
        assert_eq!(
            configuration_base("command line", Path::new("/workspace")),
            Path::new("/workspace")
        );
    }

    #[test]
    fn leaves_bare_program_names_for_path_lookup() {
        assert_eq!(
            resolve_program_path(OsString::from("custom-rustc"), Path::new("/workspace")),
            OsString::from("custom-rustc")
        );
        assert_eq!(
            resolve_program_path(OsString::from("tools/rustc"), Path::new("/workspace")),
            OsString::from(PathBuf::from("/workspace/tools/rustc"))
        );
    }

    #[test]
    fn explicit_compiler_environment_overrides_cargo_configuration() {
        let configured = ConfiguredProgram {
            value: OsString::from("configured-rustc"),
        };

        assert_eq!(
            effective_program_from(
                Some(OsString::from("tools/rustc")),
                Some(&configured),
                "rustc",
                Path::new("/workspace")
            ),
            OsString::from("/workspace/tools/rustc")
        );
        assert_eq!(
            effective_wrapper_from(
                Some(OsString::new()),
                Some(&configured),
                Path::new("/workspace")
            ),
            None
        );
    }

    #[test]
    fn accepts_a_complete_stable_compiler_identity() {
        let verbose = "rustc 1.97.1\nrelease: 1.97.1\ncommit-hash: abc123\nhost: test-host\nLLVM version: 22.1.6\n";

        assert_eq!(
            field(verbose, "release").expect("the stable release is complete"),
            "1.97.1"
        );
        assert_eq!(
            field(verbose, "commit-hash").expect("the stable commit is complete"),
            "abc123"
        );
    }

    #[test]
    fn identifies_only_proven_rustup_sysroots() {
        assert_eq!(
            rustup_toolchain_name(Path::new("/home/user/.rustup/toolchains/1.97.1-test-host")),
            Some("1.97.1-test-host".to_owned())
        );
        assert_eq!(rustup_toolchain_name(Path::new("/opt/rust")), None);
    }
}
