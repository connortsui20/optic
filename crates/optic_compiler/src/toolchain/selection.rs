//! Resolves the complete rustc invocation selected by Cargo.
//!
//! Cargo selects rustc, an outer wrapper, and a workspace wrapper independently. Environment
//! variables take precedence over file-backed configuration, and an empty wrapper environment
//! variable explicitly disables its configured wrapper. File-backed executable paths are resolved
//! relative to the directory above `.cargo`, matching Cargo's path rules.

use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use optic_records::RustcInvocation;
use snafu::ResultExt;

use crate::CargoConfigurationSnafu;
use crate::Error;
use crate::InvalidCargoConfigurationValueSnafu;
use crate::MissingCargoConfigurationOriginSnafu;
use crate::MissingCargoConfigurationSnafu;
use crate::StartProcessSnafu;

const RUSTC_CONFIG: &str = "build.rustc";
const RUSTC_WRAPPER_CONFIG: &str = "build.rustc-wrapper";
const RUSTC_WORKSPACE_WRAPPER_CONFIG: &str = "build.rustc-workspace-wrapper";

pub(crate) fn rustc_invocation(
    cargo: &Path,
    workspace_root: &Path,
) -> Result<RustcInvocation, Error> {
    let rustc = selected_rustc(cargo, workspace_root)?;
    let rustc_wrapper =
        selected_wrapper(cargo, workspace_root, "RUSTC_WRAPPER", RUSTC_WRAPPER_CONFIG)?;
    let rustc_workspace_wrapper = selected_wrapper(
        cargo,
        workspace_root,
        "RUSTC_WORKSPACE_WRAPPER",
        RUSTC_WORKSPACE_WRAPPER_CONFIG,
    )?;

    Ok(RustcInvocation::new(
        rustc,
        rustc_wrapper,
        rustc_workspace_wrapper,
    )?)
}

fn selected_rustc(cargo: &Path, workspace_root: &Path) -> Result<PathBuf, Error> {
    if let Some(rustc) = env::var_os("RUSTC").filter(|value| !value.is_empty()) {
        return Ok(resolve_program_path(rustc, workspace_root));
    }

    Ok(configured_program(cargo, workspace_root, RUSTC_CONFIG)?
        .filter(|program| !program.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("rustc")))
}

fn selected_wrapper(
    cargo: &Path,
    workspace_root: &Path,
    environment_variable: &'static str,
    configuration_key: &'static str,
) -> Result<Option<PathBuf>, Error> {
    if let Some(wrapper) = env::var_os(environment_variable) {
        if wrapper.is_empty() {
            return Ok(None);
        }

        return Ok(Some(resolve_program_path(wrapper, workspace_root)));
    }

    Ok(
        configured_program(cargo, workspace_root, configuration_key)?
            .filter(|program| !program.as_os_str().is_empty()),
    )
}

fn configured_program(
    cargo: &Path,
    workspace_root: &Path,
    key: &'static str,
) -> Result<Option<PathBuf>, Error> {
    // `cargo config get` is unstable. Restrict `RUSTC_BOOTSTRAP` to this read-only query so it
    // cannot affect the build whose compiler identity is recorded.
    let output = Command::new(cargo)
        .current_dir(workspace_root)
        .args([
            "-Z",
            "unstable-options",
            "config",
            "get",
            key,
            "--show-origin",
        ])
        .env("RUSTC_BOOTSTRAP", "1")
        .output()
        .with_context(|_| StartProcessSnafu {
            program: cargo.to_owned(),
        })?;
    if output.status.success() {
        return parse_configured_program(
            &String::from_utf8_lossy(&output.stdout),
            key,
            workspace_root,
        )
        .map(Some);
    }

    let diagnostics = String::from_utf8_lossy(&output.stderr);
    if diagnostics.contains(&format!("config value `{key}` is not set")) {
        return Ok(None);
    }

    CargoConfigurationSnafu {
        key,
        diagnostics: diagnostics.trim().to_owned(),
    }
    .fail()
}

fn parse_configured_program(
    output: &str,
    key: &'static str,
    workspace_root: &Path,
) -> Result<PathBuf, Error> {
    let prefix = format!("{key} = ");
    let line = output
        .lines()
        .find(|line| line.starts_with(&prefix))
        .ok_or_else(|| MissingCargoConfigurationSnafu { key }.build())?;
    let remainder = line
        .strip_prefix(&prefix)
        .expect("the selected Cargo output line has the required configuration-key prefix");
    let (encoded_value, origin) = remainder
        .rsplit_once(" # ")
        .ok_or_else(|| MissingCargoConfigurationOriginSnafu { key, line }.build())?;
    let value = serde_json::from_str::<String>(encoded_value)
        .with_context(|_| InvalidCargoConfigurationValueSnafu { key, encoded_value })?;
    let base = configuration_base(origin, workspace_root);

    Ok(resolve_program_path(OsString::from(value), base))
}

fn configuration_base<'a>(origin: &'a str, workspace_root: &'a Path) -> &'a Path {
    if origin.starts_with("environment variable `") || origin.starts_with("command line") {
        return workspace_root;
    }

    // Cargo resolves executable paths from configuration files relative to the directory above
    // `.cargo`. See https://doc.rust-lang.org/cargo/reference/config.html#config-relative-paths.
    Path::new(origin)
        .parent()
        .and_then(Path::parent)
        .unwrap_or(workspace_root)
}

fn resolve_program_path(value: OsString, base: &Path) -> PathBuf {
    let path = Path::new(&value);
    let has_separator = path.components().count() > 1
        || path
            .as_os_str()
            .to_string_lossy()
            .contains(std::path::MAIN_SEPARATOR);
    if path.is_absolute() || !has_separator {
        return PathBuf::from(value);
    }

    base.join(path)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use super::parse_configured_program;

    #[test]
    fn resolves_each_program_from_its_cargo_configuration_origin() {
        let cases = [
            (
                "build.rustc", // Configuration key.
                "build.rustc = \"tools/rustc\" # /workspace/.cargo/config.toml\n",
                PathBuf::from("/workspace/tools/rustc"),
            ),
            (
                "build.rustc-wrapper", // Configuration key.
                "build.rustc-wrapper = \"wrapper\" # environment variable `CARGO_BUILD_RUSTC_WRAPPER`\n",
                PathBuf::from("wrapper"),
            ),
            (
                "build.rustc-workspace-wrapper", // Configuration key.
                "build.rustc-workspace-wrapper = \"tools/workspace\" # command line\n",
                PathBuf::from("/workspace/tools/workspace"),
            ),
        ];

        for (key, output, expected) in cases {
            let program = parse_configured_program(output, key, Path::new("/workspace"))
                .expect("the fixture contains a complete Cargo configuration value");

            assert_eq!(program, expected);
        }
    }
}
