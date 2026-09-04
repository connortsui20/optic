//! Builds and configures the exact-version rustc driver.
//!
//! Cargo metadata describes packages and targets, but it does not expose the concrete generic
//! functions, symbols, or codegen-unit placements that rustc produces. Optic gets that evidence
//! through a small standalone program that uses rustc's internal compiler crates.
//!
//! The standalone program is source data at `../rustc-driver/main.rs`, not a child module of this
//! crate. [`RustcDriver::build`] writes that source to the collection's temporary directory and
//! compiles it with the rustc selected by Cargo. This exact-version build is required because
//! rustc's internal crates have no stable API or cross-version compatibility contract. The
//! selected toolchain's `rustc-dev` component provides the internal libraries and metadata needed
//! to compile the program.
//!
//! [`RustcDriver::configure`] then installs the compiled program as Cargo's outer global wrapper.
//! Existing transparent forwarding wrappers retain their Cargo-defined positions. Driver-style
//! wrappers that interpret rustc's command line are unsupported because Optic must replace rustc
//! for the selected target. The Optic wrapper passes every unrelated compiler invocation through
//! and enters the internal compiler driver only for the selected target.

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
#[cfg(windows)]
use crate::error::CompilerEnvironmentSnafu;
use crate::protocol::DRIVER_INNER_ENV;
use crate::protocol::MANIFEST_PATH_ENV;
use crate::protocol::ORIGINAL_WRAPPER_ENV;
use crate::protocol::RUSTC_COMMAND_ENV;
use crate::protocol::RUSTC_COMMIT_ENV;
use crate::protocol::RUSTC_HOST_ENV;
use crate::protocol::RUSTC_PATH_ENV;
use crate::protocol::RUSTC_RELEASE_ENV;
use crate::protocol::RUSTC_SYSROOT_ENV;
use crate::protocol::SELECTED_TARGET_MARKER_ENV;
use crate::protocol::WORKSPACE_WRAPPER_ENV;
use crate::protocol::WRAPPER_ACTIVE_ENV;
use crate::toolchain::CompilerContext;

// The main source is compiled separately with the selected rustc. It must not become a module of
// this crate because that would bind the crate itself to one build-time version of rustc's
// internals. The pure support modules compile in both contexts so their contracts stay
// synchronized.
const DRIVER_SOURCE: &str = include_str!("../rustc-driver/main.rs");
const ARGUMENTS_SOURCE: &str = include_str!("../rustc-driver/arguments.rs");
const PROTOCOL_SOURCE: &str = include_str!("../rustc-driver/protocol.rs");

/// A temporary standalone driver prepared for one selected compiler.
pub(crate) struct RustcDriver {
    /// The driver executable compiled with the selected rustc.
    executable: PathBuf,
    /// The global wrapper that the Optic wrapper temporarily displaces.
    original_global_wrapper: Option<OsString>,
    /// Whether Cargo must retain a workspace wrapper inside the Optic wrapper.
    has_workspace_wrapper: bool,
}

impl RustcDriver {
    /// Compiles the standalone driver with the selected rustc and retains Cargo's wrapper chain.
    pub(crate) fn build(
        workspace: &Workspace,
        compiler: &CompilerContext,
        directory: &Path,
    ) -> Result<Self, Error> {
        if env::var_os(WRAPPER_ACTIVE_ENV).is_some() {
            return Err(Error::CompilerEnvironment {
                message: "a Cargo Optic rustc wrapper is already active".to_owned(),
            });
        }

        require_rustc_dev(
            compiler.identity(),
            compiler.rustc_private_library_directory(),
            compiler.rustup_toolchain(),
        )?;

        let sources = [
            ("optic-rustc-driver.rs", DRIVER_SOURCE),
            ("arguments.rs", ARGUMENTS_SOURCE),
            ("protocol.rs", PROTOCOL_SOURCE),
        ];
        for (name, contents) in sources {
            let path = directory.join(name);
            fs::write(&path, contents).map_err(|source| Error::Filesystem {
                operation: "write rustc driver source to",
                path,
                source,
            })?;
        }
        let source = directory.join("optic-rustc-driver.rs");
        let executable = directory.join(executable_name());
        build_driver(workspace, compiler.rustc_command(), &source, &executable)?;

        Ok(Self {
            executable,
            original_global_wrapper: compiler.global_wrapper().map(OsStr::to_owned),
            has_workspace_wrapper: compiler.workspace_wrapper().is_some(),
        })
    }

    /// Installs this driver as Cargo's outer wrapper for one selected-target invocation.
    pub(crate) fn configure(
        &self,
        command: &mut Command,
        compiler: &CompilerContext,
        selected_target_marker: &str,
        manifest_path: &Path,
    ) -> Result<(), Error> {
        let identity = compiler.identity();

        command
            .env("RUSTC_WRAPPER", &self.executable)
            .env(WRAPPER_ACTIVE_ENV, "1")
            .env(SELECTED_TARGET_MARKER_ENV, selected_target_marker)
            .env(MANIFEST_PATH_ENV, manifest_path)
            .env(RUSTC_COMMAND_ENV, compiler.rustc_command())
            .env(RUSTC_PATH_ENV, identity.rustc())
            .env(RUSTC_RELEASE_ENV, identity.release())
            .env(RUSTC_COMMIT_ENV, identity.commit_hash())
            .env(RUSTC_HOST_ENV, identity.host())
            .env(RUSTC_SYSROOT_ENV, identity.sysroot())
            .env_remove(DRIVER_INNER_ENV);

        #[cfg(windows)]
        configure_windows_rustc_private_search_path(command, identity)?;

        match &self.original_global_wrapper {
            Some(wrapper) => {
                command.env(ORIGINAL_WRAPPER_ENV, wrapper);
            }
            None => {
                command.env_remove(ORIGINAL_WRAPPER_ENV);
            }
        }

        if self.has_workspace_wrapper {
            command.env(WORKSPACE_WRAPPER_ENV, "1");
        } else {
            command.env_remove(WORKSPACE_WRAPPER_ENV);
        }

        Ok(())
    }
}

/// Makes the selected rustc-private DLLs visible to the Windows loader.
///
/// The driver links dynamically to compiler libraries from the selected sysroot. On Windows, the
/// operating-system loader resolves those DLLs before the wrapper's `main` function starts. The
/// driver lives in a temporary directory, outside the selected sysroot, so the wrapper cannot
/// repair its own search path after startup.
///
/// The [Windows DLL search order] includes the child process's `PATH`. Cargo starts the wrapper, so
/// this function prepends the selected sysroot's `bin` directory to Cargo's child `PATH`.
/// Prepending, instead of appending, prevents a different toolchain's DLL with the same name from
/// taking precedence.
///
/// # Errors
///
/// Returns an error if the selected sysroot and inherited `PATH` cannot form a valid Windows search
/// path.
///
/// [Windows DLL search order]: https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order
#[cfg(windows)]
fn configure_windows_rustc_private_search_path(
    command: &mut Command,
    compiler: &CompilerIdentity,
) -> Result<(), Error> {
    let mut search_paths = vec![compiler.sysroot().join("bin")];
    if let Some(existing) = env::var_os("PATH") {
        search_paths.extend(env::split_paths(&existing));
    }
    let search_path = env::join_paths(search_paths).map_err(|error| {
        CompilerEnvironmentSnafu {
            message: format!(
                "Windows driver search path entries must be representable in PATH, got {error}"
            ),
        }
        .build()
    })?;

    command.env("PATH", search_path);

    Ok(())
}

fn require_rustc_dev(
    compiler: &CompilerIdentity,
    rustc_private_library_directory: &Path,
    rustup_toolchain: Option<&str>,
) -> Result<(), Error> {
    let missing = || Error::MissingRustcDev {
        release: compiler.release().to_owned(),
        commit_hash: compiler.commit_hash().to_owned(),
        path: rustc_private_library_directory.to_owned(),
        install_command: rustup_toolchain
            .map(|toolchain| format!("rustup component add --toolchain {toolchain} rustc-dev")),
    };
    let entries = match fs::read_dir(rustc_private_library_directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Err(missing()),
        Err(source) => {
            return Err(Error::Filesystem {
                operation: "read rustc-private library directory",
                path: rustc_private_library_directory.to_owned(),
                source,
            });
        }
    };

    for entry in entries {
        let entry = entry.map_err(|source| Error::Filesystem {
            operation: "read entry in rustc-private library directory",
            path: rustc_private_library_directory.to_owned(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("librustc_middle-")
            && (name.ends_with(".rlib") || name.ends_with(".rmeta"))
        {
            return Ok(());
        }
    }

    Err(missing())
}

fn build_driver(
    workspace: &Workspace,
    rustc: &OsStr,
    source: &Path,
    executable: &Path,
) -> Result<(), Error> {
    // Scope unstable compiler-library access to the temporary driver crate. The later Cargo command
    // removes `RUSTC_BOOTSTRAP` from its environment.
    let output = Command::new(rustc)
        .current_dir(workspace.invocation_directory())
        .arg(source)
        .args(["--crate-name", "optic_rustc_driver", "--edition=2024"])
        .args(["-C", "prefer-dynamic", "-C", "rpath"])
        .arg("-o")
        .arg(executable)
        .env("RUSTC_BOOTSTRAP", "optic_rustc_driver")
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

    Ok(())
}

fn executable_name() -> OsString {
    let mut name = OsString::from("optic-rustc-driver");
    name.push(env::consts::EXE_SUFFIX);

    name
}

#[cfg(test)]
mod tests {
    use optic_records::CompilerIdentity;

    use super::require_rustc_dev;

    #[test]
    fn reports_an_actionable_missing_rustc_dev_component() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let rustc_private_library_directory = temporary.path().join("missing-rustc-private");
        let compiler = CompilerIdentity::new(
            temporary.path().join("rustc"),
            "1.98.0",
            "abc123",
            "test-host",
            temporary.path().join("toolchains/test-toolchain"),
        )
        .expect("the fixture compiler identity is valid");

        let error = require_rustc_dev(
            &compiler,
            &rustc_private_library_directory,
            Some("test-toolchain"),
        )
        .expect_err("the missing rustc-dev component must be rejected");
        let message = error.to_string();

        assert!(message.contains("rustc 1.98.0 (abc123)"));
        assert!(message.contains(&rustc_private_library_directory.display().to_string()));
        assert!(message.contains("rustup component add --toolchain test-toolchain rustc-dev"));
    }
}
