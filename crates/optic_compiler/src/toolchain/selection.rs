//! Resolves the rustc executable selected by Cargo configuration.
//!
//! Environment selection is relative to the workspace. File-backed Cargo configuration resolves
//! relative paths from the directory above `.cargo`, matching Cargo's own path rules.

use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use snafu::ResultExt;

use crate::CargoConfigurationSnafu;
use crate::Error;
use crate::InvalidCargoConfigurationValueSnafu;
use crate::MissingCargoConfigurationOriginSnafu;
use crate::MissingCargoConfigurationSnafu;
use crate::StartProcessSnafu;

pub(crate) fn selected_rustc(cargo: &Path, workspace_root: &Path) -> Result<PathBuf, Error> {
    if let Some(rustc) = env::var_os("RUSTC").filter(|value| !value.is_empty()) {
        return Ok(resolve_program_path(rustc, workspace_root));
    }

    configured_rustc(cargo, workspace_root)
}

fn configured_rustc(cargo: &Path, workspace_root: &Path) -> Result<PathBuf, Error> {
    // `cargo config get` is unstable. Restrict `RUSTC_BOOTSTRAP` to this read-only query so it
    // cannot affect the build whose compiler identity is recorded.
    let output = Command::new(cargo)
        .current_dir(workspace_root)
        .args([
            "-Z",
            "unstable-options",
            "config",
            "get",
            "build.rustc",
            "--show-origin",
        ])
        .env("RUSTC_BOOTSTRAP", "1")
        .output()
        .with_context(|_| StartProcessSnafu {
            program: cargo.to_owned(),
        })?;
    if output.status.success() {
        return parse_configured_rustc(&String::from_utf8_lossy(&output.stdout), workspace_root);
    }

    let diagnostics = String::from_utf8_lossy(&output.stderr);
    if diagnostics.contains("config value `build.rustc` is not set") {
        return Ok(PathBuf::from("rustc"));
    }

    CargoConfigurationSnafu {
        diagnostics: diagnostics.trim().to_owned(),
    }
    .fail()
}

fn parse_configured_rustc(output: &str, workspace_root: &Path) -> Result<PathBuf, Error> {
    let line = output
        .lines()
        .find(|line| line.starts_with("build.rustc = "))
        .ok_or_else(|| MissingCargoConfigurationSnafu.build())?;
    let remainder = line
        .strip_prefix("build.rustc = ")
        .expect("the selected Cargo output line starts with the build.rustc prefix");
    let (encoded_value, origin) = remainder
        .rsplit_once(" # ")
        .ok_or_else(|| MissingCargoConfigurationOriginSnafu { line }.build())?;
    let value = serde_json::from_str::<String>(encoded_value)
        .with_context(|_| InvalidCargoConfigurationValueSnafu { encoded_value })?;
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

    use super::parse_configured_rustc;

    #[test]
    fn resolves_program_paths_from_cargo_configuration_origins() {
        let file = parse_configured_rustc(
            "build.rustc = \"tools/rustc\" # /workspace/.cargo/config.toml\n",
            Path::new("/other"),
        )
        .expect("the file-origin fixture contains a complete Cargo configuration value");
        let environment = parse_configured_rustc(
            "build.rustc = \"tools/rustc\" # environment variable `CARGO_BUILD_RUSTC`\n",
            Path::new("/workspace"),
        )
        .expect("the environment-origin fixture contains a complete Cargo configuration value");

        assert_eq!(file, PathBuf::from("/workspace/tools/rustc"));
        assert_eq!(environment, PathBuf::from("/workspace/tools/rustc"));
    }
}
