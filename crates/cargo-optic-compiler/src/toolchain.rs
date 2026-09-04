//! Resolves the compiler and wrapper chain selected by Cargo.
//!
//! Cargo configuration, environment variables, and the invocation directory can all change the
//! selected compiler. Collection resolves them together so driver compilation and the subsequent
//! `cargo rustc` invocation start from the same workspace context.
//!
//! Compiler command classification rejects front-ends that the in-process driver would bypass. It
//! protects replacement fidelity, not authenticity: the selected executable can still lie about
//! its identity and sysroot.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use optic_records::CompilerIdentity;

use crate::Error;
use crate::Workspace;

const CONFIG_FIELDS: [&str; 3] = [
    "build.rustc",                   // Compiler command.
    "build.rustc-wrapper",           // Global compiler wrapper.
    "build.rustc-workspace-wrapper", // Workspace compiler wrapper.
];

/// The compiler command and matching internal libraries selected by Cargo.
///
/// The rustc-private library directory comes from this compiler's reported sysroot. The standalone
/// driver must compile against that exact directory because rustc's internal crates do not promise
/// compatibility between toolchain versions.
pub(crate) struct CompilerContext {
    /// The identity reported by the actual compiler in the selected sysroot.
    identity: CompilerIdentity,

    /// The Cargo-selected command, which can be the compiler itself or a rustup proxy.
    rustc_command: OsString,

    /// The host library directory populated by the matching `rustc-dev` component.
    rustc_private_library_directory: PathBuf,

    /// The rustup toolchain name proven from the selected sysroot layout.
    rustup_toolchain: Option<String>,

    /// The global wrapper displaced when Optic becomes Cargo's outer wrapper.
    global_wrapper: Option<OsString>,

    /// The workspace wrapper retained inside the global wrapper.
    workspace_wrapper: Option<OsString>,
}

impl CompilerContext {
    /// Resolves Cargo's effective compiler configuration and verifies the selected rustc command.
    pub(crate) fn discover(workspace: &Workspace) -> Result<Self, Error> {
        let configuration = CargoConfiguration::read(workspace)?;
        let configured_rustc = configuration.rustc.as_ref();
        let rustc = effective_program(
            "RUSTC",
            "CARGO_BUILD_RUSTC",
            configured_rustc,
            OsStr::new("rustc"),
            workspace.invocation_directory(),
        );
        let rustc = resolve_executable(&rustc, workspace.invocation_directory())?;
        let identity = inspect_rustc(rustc.as_os_str(), workspace.invocation_directory())?;
        classify_rustc_command(&rustc, identity.rustc())?;
        let rustc_private_library_directory = identity
            .sysroot()
            .join("lib")
            .join("rustlib")
            .join(identity.host())
            .join("lib");
        let rustup_toolchain = rustup_toolchain_name(identity.sysroot());

        let global_wrapper = effective_wrapper(
            "RUSTC_WRAPPER",
            "CARGO_BUILD_RUSTC_WRAPPER",
            configuration.global_wrapper.as_ref(),
            workspace.invocation_directory(),
        );
        let workspace_wrapper = effective_wrapper(
            "RUSTC_WORKSPACE_WRAPPER",
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
            configuration.workspace_wrapper.as_ref(),
            workspace.invocation_directory(),
        );

        Ok(Self {
            identity,
            rustc_command: rustc.into_os_string(),
            rustc_private_library_directory,
            rustup_toolchain,
            global_wrapper,
            workspace_wrapper,
        })
    }

    pub(crate) fn identity(&self) -> &CompilerIdentity {
        &self.identity
    }

    pub(crate) fn rustc_command(&self) -> &OsStr {
        &self.rustc_command
    }

    pub(crate) fn rustc_private_library_directory(&self) -> &Path {
        &self.rustc_private_library_directory
    }

    pub(crate) fn rustup_toolchain(&self) -> Option<&str> {
        self.rustup_toolchain.as_deref()
    }

    pub(crate) fn global_wrapper(&self) -> Option<&OsStr> {
        self.global_wrapper.as_deref()
    }

    pub(crate) fn workspace_wrapper(&self) -> Option<&OsStr> {
        self.workspace_wrapper.as_deref()
    }
}

/// The compiler and wrappers from Cargo's merged configuration.
#[derive(Default)]
struct CargoConfiguration {
    rustc: Option<ConfiguredProgram>,
    global_wrapper: Option<ConfiguredProgram>,
    workspace_wrapper: Option<ConfiguredProgram>,
}

impl CargoConfiguration {
    fn read(workspace: &Workspace) -> Result<Self, Error> {
        // `cargo config get` is unstable. This bootstrap value applies only to the short-lived
        // Cargo configuration query; it is not inherited by the selected target compilation.
        // See <https://doc.rust-lang.org/cargo/commands/cargo-config.html>.
        let output = Command::new(workspace.cargo())
            .current_dir(workspace.invocation_directory())
            .args(["-Z", "unstable-options", "config", "get", "--show-origin"])
            .env("RUSTC_BOOTSTRAP", "1")
            .output()
            .map_err(|source| Error::StartProcess {
                program: workspace.cargo().to_owned(),
                source,
            })?;
        if !output.status.success() {
            let diagnostics = String::from_utf8_lossy(&output.stderr);
            return Err(Error::ProcessFailed {
                program: workspace.cargo().to_owned(),
                status: output.status.to_string(),
                diagnostics: Some(diagnostics.into_owned()),
            });
        }

        Self::parse(&String::from_utf8_lossy(&output.stdout), workspace.root())
    }

    fn parse(output: &str, workspace_root: &Path) -> Result<Self, Error> {
        let mut configuration = Self::default();

        for line in output.lines() {
            let Some((field, program)) = parse_configuration_line(line, workspace_root)? else {
                continue;
            };

            match field {
                "build.rustc" => configuration.rustc = Some(program),
                "build.rustc-wrapper" => configuration.global_wrapper = Some(program),
                "build.rustc-workspace-wrapper" => configuration.workspace_wrapper = Some(program),
                _ => unreachable!("CONFIG_FIELDS contains every matched field"),
            }
        }

        Ok(configuration)
    }
}

/// Parses one relevant line from Cargo's merged configuration output.
fn parse_configuration_line<'a>(
    line: &'a str,
    workspace_root: &Path,
) -> Result<Option<(&'a str, ConfiguredProgram)>, Error> {
    let Some((field, remainder)) = line.split_once(" = ") else {
        return Ok(None);
    };
    if !CONFIG_FIELDS.contains(&field) {
        return Ok(None);
    }

    let Some((encoded_value, origin)) = remainder.rsplit_once(" # ") else {
        return Err(Error::CompilerEnvironment {
            message: format!("Cargo must report an origin for {field}, got {line}"),
        });
    };
    let document = format!("value = {encoded_value}");
    let value = document
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut table| table.remove("value"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| Error::CompilerEnvironment {
            message: format!("Cargo must return a string for {field}, got {encoded_value}"),
        })?;

    Ok(Some((
        field,
        ConfiguredProgram::new(value, origin, workspace_root),
    )))
}

/// One Cargo-configured program resolved relative to the configuration source that defined it.
struct ConfiguredProgram {
    /// The command after resolving path-like values against their configuration origin.
    value: OsString,
}

impl ConfiguredProgram {
    fn new(value: String, origin: &str, workspace_root: &Path) -> Self {
        let base = configuration_base(origin, workspace_root);
        let value = resolve_program_path(OsString::from(value), base);

        Self { value }
    }
}

fn effective_program(
    direct_environment_name: &str,
    cargo_environment_name: &str,
    configured: Option<&ConfiguredProgram>,
    default: &OsStr,
    invocation_directory: &Path,
) -> OsString {
    effective_program_value(
        env::var_os(direct_environment_name),
        env::var_os(cargo_environment_name),
        configured,
        default,
        invocation_directory,
    )
}

fn effective_program_value(
    direct_environment: Option<OsString>,
    cargo_environment: Option<OsString>,
    configured: Option<&ConfiguredProgram>,
    default: &OsStr,
    invocation_directory: &Path,
) -> OsString {
    if let Some(value) = direct_environment.or(cargo_environment) {
        return resolve_program_path(value, invocation_directory);
    }

    configured.map_or_else(|| default.to_owned(), |program| program.value.clone())
}

fn effective_wrapper(
    direct_environment_name: &str,
    cargo_environment_name: &str,
    configured: Option<&ConfiguredProgram>,
    invocation_directory: &Path,
) -> Option<OsString> {
    effective_wrapper_value(
        env::var_os(direct_environment_name),
        env::var_os(cargo_environment_name),
        configured,
        invocation_directory,
    )
}

fn effective_wrapper_value(
    direct_environment: Option<OsString>,
    cargo_environment: Option<OsString>,
    configured: Option<&ConfiguredProgram>,
    invocation_directory: &Path,
) -> Option<OsString> {
    match direct_environment.or(cargo_environment) {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(resolve_program_path(value, invocation_directory)),
        None => configured
            .filter(|program| !program.value.is_empty())
            .map(|program| program.value.clone()),
    }
}

fn configuration_base<'a>(origin: &'a str, workspace_root: &'a Path) -> &'a Path {
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

fn resolve_executable(command: &OsStr, directory: &Path) -> Result<PathBuf, Error> {
    which::which_in(command, env::var_os("PATH"), directory).map_err(|_| {
        Error::CompilerEnvironment {
            message: format!(
                "selected compiler command must resolve to an executable, got {}",
                Path::new(command).display()
            ),
        }
    })
}

fn classify_rustc_command(command: &Path, sysroot_rustc: &Path) -> Result<(), Error> {
    if same_file::is_same_file(command, sysroot_rustc).map_err(|source| Error::Filesystem {
        operation: "compare selected compiler executable with",
        path: sysroot_rustc.to_owned(),
        source,
    })? {
        return Ok(());
    }

    if is_rustup_proxy(command)? {
        return Ok(());
    }

    Err(Error::CompilerEnvironment {
        message: format!(
            "selected compiler command must be the reported sysroot rustc or a rustup proxy; custom compiler front-ends are not supported, got {}",
            command.display()
        ),
    })
}

fn is_rustup_proxy(command: &Path) -> Result<bool, Error> {
    let rustc_name = format!("rustc{}", env::consts::EXE_SUFFIX);
    if command.file_name() != Some(OsStr::new(&rustc_name)) {
        return Ok(false);
    }

    let rustup = command.with_file_name(format!("rustup{}", env::consts::EXE_SUFFIX));
    match same_file::is_same_file(command, &rustup) {
        Ok(is_same_file) => Ok(is_same_file),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(Error::Filesystem {
            operation: "inspect rustup proxy executable",
            path: rustup,
            source,
        }),
    }
}

fn inspect_rustc(rustc: &OsStr, directory: &Path) -> Result<CompilerIdentity, Error> {
    let verbose = run_rustc(rustc, directory, &["-vV"])?;
    let release = compiler_field(&verbose, "release")?;
    let commit_hash = compiler_field(&verbose, "commit-hash")?;
    let host = compiler_field(&verbose, "host")?;
    let reported_sysroot = run_rustc(rustc, directory, &["--print", "sysroot"])?;
    let reported_sysroot = PathBuf::from(reported_sysroot.trim());
    let sysroot = fs::canonicalize(&reported_sysroot).map_err(|source| Error::Filesystem {
        operation: "resolve selected rustc sysroot",
        path: reported_sysroot,
        source,
    })?;
    let actual_rustc = actual_rustc_path(&sysroot)?;

    CompilerIdentity::new(actual_rustc, release, commit_hash, host, sysroot).map_err(Error::from)
}

fn actual_rustc_path(sysroot: &Path) -> Result<PathBuf, Error> {
    let executable = format!("rustc{}", env::consts::EXE_SUFFIX);
    let sysroot_rustc = sysroot.join("bin").join(executable);
    let actual_rustc = fs::canonicalize(&sysroot_rustc).map_err(|source| Error::Filesystem {
        operation: "inspect selected sysroot rustc executable",
        path: sysroot_rustc,
        source,
    })?;

    Ok(actual_rustc)
}

fn run_rustc(rustc: &OsStr, directory: &Path, arguments: &[&str]) -> Result<String, Error> {
    let output = Command::new(rustc)
        .current_dir(directory)
        .args(arguments)
        .output()
        .map_err(|source| Error::StartProcess {
            program: PathBuf::from(rustc),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: PathBuf::from(rustc),
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
            message: format!("selected rustc -vV must report {name}, got no value"),
        });
    };

    Ok(value.to_owned())
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
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::path::Path;

    use super::CargoConfiguration;
    use super::ConfiguredProgram;
    use super::classify_rustc_command;
    use super::effective_program_value;
    use super::effective_wrapper_value;
    use super::rustup_toolchain_name;

    #[test]
    fn parses_cargo_programs_with_relative_path_origins() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let workspace = temporary.path().join("workspace");
        let configuration_path = workspace.join(".cargo/config.toml");
        let output = format!(
            r#"
build.rustc = "tools/rustc" # {}
build.rustc-wrapper = "global-wrapper" # user configuration
build.rustc-workspace-wrapper = "tools/workspace-wrapper" # {}
"#,
            configuration_path.display(),
            configuration_path.display()
        );

        let configuration = CargoConfiguration::parse(&output, &workspace)
            .expect("the Cargo configuration is valid");

        assert_eq!(
            configuration.rustc.expect("rustc is configured").value,
            workspace.join("tools/rustc").into_os_string()
        );
        assert_eq!(
            configuration
                .global_wrapper
                .expect("the global wrapper is configured")
                .value,
            OsString::from("global-wrapper")
        );
        assert_eq!(
            configuration
                .workspace_wrapper
                .expect("the workspace wrapper is configured")
                .value,
            workspace.join("tools/workspace-wrapper").into_os_string()
        );
    }

    #[test]
    fn parses_cargo_literal_string_programs() {
        let workspace = Path::new("/workspace");
        let output = r"build.rustc-wrapper = 'C:\tools\sccache.exe' # user configuration";

        let configuration = CargoConfiguration::parse(output, workspace)
            .expect("the Cargo literal string is valid");

        assert_eq!(
            configuration
                .global_wrapper
                .expect("the global wrapper is configured")
                .value,
            OsString::from(r"C:\tools\sccache.exe")
        );
    }

    #[test]
    fn identifies_only_proven_rustup_sysroots() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");

        assert_eq!(
            rustup_toolchain_name(&temporary.path().join("toolchains/stable-host")),
            Some("stable-host".to_owned())
        );
        assert_eq!(rustup_toolchain_name(&temporary.path().join("rust")), None);
    }

    #[test]
    fn applies_cargo_compiler_environment_precedence() {
        let configured = ConfiguredProgram {
            value: OsString::from("configured-rustc"),
        };
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let invocation_directory = temporary.path().join("workspace/member");

        assert_eq!(
            effective_program_value(
                Some(OsString::from("direct-rustc")),
                Some(OsString::from("cargo-rustc")),
                Some(&configured),
                OsStr::new("rustc"),
                &invocation_directory,
            ),
            OsString::from("direct-rustc")
        );
        assert_eq!(
            effective_program_value(
                None,
                Some(OsString::from("tools/cargo-rustc")),
                Some(&configured),
                OsStr::new("rustc"),
                &invocation_directory,
            ),
            invocation_directory
                .join("tools/cargo-rustc")
                .into_os_string()
        );
    }

    #[test]
    fn applies_cargo_wrapper_environment_precedence() {
        let configured = ConfiguredProgram {
            value: OsString::from("configured-wrapper"),
        };
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let invocation_directory = temporary.path().join("workspace/member");

        assert_eq!(
            effective_wrapper_value(
                Some(OsString::from("direct-wrapper")),
                Some(OsString::from("cargo-wrapper")),
                Some(&configured),
                &invocation_directory,
            ),
            Some(OsString::from("direct-wrapper"))
        );
        assert_eq!(
            effective_wrapper_value(
                None,
                Some(OsString::from("tools/cargo-wrapper")),
                Some(&configured),
                &invocation_directory,
            ),
            Some(
                invocation_directory
                    .join("tools/cargo-wrapper")
                    .into_os_string()
            )
        );
        assert_eq!(
            effective_wrapper_value(
                Some(OsString::new()),
                Some(OsString::from("cargo-wrapper")),
                Some(&configured),
                &invocation_directory,
            ),
            None
        );
    }

    #[test]
    fn treats_an_empty_configured_wrapper_as_disabled() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let invocation_directory = temporary.path();
        let configured = ConfiguredProgram {
            value: OsString::new(),
        };

        let wrapper = effective_wrapper_value(None, None, Some(&configured), invocation_directory);

        assert_eq!(wrapper, None);
    }

    #[test]
    fn classifies_the_sysroot_compiler_by_file_identity() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let rustc = temporary.path().join("command/rustc");
        let sysroot_rustc = temporary.path().join("toolchain/bin/rustc");
        std::fs::create_dir_all(rustc.parent().expect("rustc has a parent"))
            .expect("the test can create the command directory");
        std::fs::create_dir_all(sysroot_rustc.parent().expect("rustc has a parent"))
            .expect("the test can create the sysroot binary directory");
        std::fs::write(&sysroot_rustc, [])
            .expect("the test can create the sysroot compiler executable");
        std::fs::hard_link(&sysroot_rustc, &rustc)
            .expect("the test can link the selected compiler executable");

        classify_rustc_command(&rustc, &sysroot_rustc)
            .expect("the sysroot compiler must be supported");
    }

    #[test]
    fn classifies_a_rustup_proxy_by_file_identity() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let rustup = temporary
            .path()
            .join(format!("rustup{}", std::env::consts::EXE_SUFFIX));
        let rustc = temporary
            .path()
            .join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
        let sysroot_rustc = temporary.path().join("toolchain/bin/rustc");
        std::fs::write(&rustup, []).expect("the test can create the rustup executable");
        std::fs::hard_link(&rustup, &rustc).expect("the test can create the rustup proxy");
        std::fs::create_dir_all(sysroot_rustc.parent().expect("rustc has a parent"))
            .expect("the test can create the sysroot binary directory");
        std::fs::write(&sysroot_rustc, [])
            .expect("the test can create the sysroot compiler executable");

        classify_rustc_command(&rustc, &sysroot_rustc).expect("the rustup proxy must be supported");
    }

    #[test]
    fn rejects_a_custom_front_end_named_rustc() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let rustc = temporary
            .path()
            .join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
        let sysroot_rustc = temporary.path().join("toolchain/bin/rustc");
        std::fs::write(&rustc, []).expect("the test can create the compiler front-end");
        std::fs::create_dir_all(sysroot_rustc.parent().expect("rustc has a parent"))
            .expect("the test can create the sysroot binary directory");
        std::fs::write(&sysroot_rustc, [])
            .expect("the test can create the sysroot compiler executable");

        let error = classify_rustc_command(&rustc, &sysroot_rustc)
            .expect_err("the custom compiler front-end must be rejected");

        assert!(error.to_string().contains(
            "selected compiler command must be the reported sysroot rustc or a rustup proxy; custom compiler front-ends are not supported"
        ));
    }
}
