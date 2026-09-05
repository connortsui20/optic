//! Builds and configures the exact-version rustc driver.
//!
//! The standalone driver uses rustc's internal crates, so each collection compiles it with the
//! default `rustc` from `PATH`. Cargo then runs it as the only compiler wrapper for the capture.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::Error;
use crate::Workspace;
use crate::protocol::DRIVER_INNER_ENV;
use crate::protocol::MANIFEST_PATH_ENV;
use crate::protocol::SELECTED_TARGET_MARKER_ENV;

const DRIVER_SOURCE: &str = include_str!("../rustc-driver/main.rs");
const ANALYSIS_SOURCE: &str = include_str!("../rustc-driver/analysis.rs");
const MANIFEST_SOURCE: &str = include_str!("../rustc-driver/manifest.rs");
const PROTOCOL_SOURCE: &str = include_str!("../rustc-driver/protocol.rs");
const WRAPPER_SOURCE: &str = include_str!("../rustc-driver/wrapper.rs");

/// A standalone driver compiled for one collection.
pub(crate) struct RustcDriver {
    executable: PathBuf,
}

impl RustcDriver {
    /// Compiles the standalone driver with `rustc` from `PATH`.
    pub(crate) fn build(workspace: &Workspace, directory: &Path) -> Result<Self, Error> {
        for (name, contents) in [
            ("optic-rustc-driver.rs", DRIVER_SOURCE),
            ("analysis.rs", ANALYSIS_SOURCE),
            ("manifest.rs", MANIFEST_SOURCE),
            ("protocol.rs", PROTOCOL_SOURCE),
            ("wrapper.rs", WRAPPER_SOURCE),
        ] {
            let path = directory.join(name);
            fs::write(&path, contents).map_err(|source| Error::Filesystem {
                operation: "write rustc driver source to",
                path,
                source,
            })?;
        }

        let source = directory.join("optic-rustc-driver.rs");
        let executable = directory.join("optic-rustc-driver");
        build_driver(workspace, &source, &executable)?;

        Ok(Self { executable })
    }

    /// Installs the driver as Cargo's only compiler wrapper for this capture.
    pub(crate) fn configure(
        &self,
        command: &mut Command,
        selected_target_marker: &str,
        manifest_path: &Path,
    ) {
        command
            .env("RUSTC_WRAPPER", &self.executable)
            .env("RUSTC_WORKSPACE_WRAPPER", "")
            .env("CARGO_BUILD_RUSTC_WRAPPER", "")
            .env("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "")
            .env(SELECTED_TARGET_MARKER_ENV, selected_target_marker)
            .env(MANIFEST_PATH_ENV, manifest_path)
            .env_remove(DRIVER_INNER_ENV);
    }
}

fn build_driver(workspace: &Workspace, source: &Path, executable: &Path) -> Result<(), Error> {
    let output = Command::new("rustc")
        .current_dir(workspace.invocation_directory())
        .arg(source)
        .args(["--crate-name", "optic_rustc_driver", "--edition=2024"])
        .args(["-C", "prefer-dynamic", "-C", "rpath"])
        .arg("-o")
        .arg(executable)
        .env("RUSTC_BOOTSTRAP", "optic_rustc_driver")
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

    Ok(())
}
