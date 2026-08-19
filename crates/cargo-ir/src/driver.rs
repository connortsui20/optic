//! Provisions the exact-version rustc driver and preserves Cargo wrapper composition.
//!
//! The driver is a small, dependency-free program embedded in `cargo-ir`. The active nightly
//! compiler builds it once for each rustc commit and source revision. Cargo then uses it as the
//! outer global wrapper without changing workspace artifact hashes.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use fs2::FileExt;
use serde_json::Value;

use crate::{Error, Result, Toolchain};

const DRIVER_SOURCE: &str = include_str!("../driver/main.rs");
const PROTOCOL_VERSION: &str = "2";

const DRIVER_INNER_ENV: &str = "OPTIC_RUSTC_DRIVER_INNER";
const MANIFEST_PATH_ENV: &str = "OPTIC_IDENTITY_MANIFEST";
const ORIGINAL_WRAPPER_ENV: &str = "OPTIC_ORIGINAL_RUSTC_WRAPPER";
const RUSTC_COMMIT_ENV: &str = "OPTIC_RUSTC_COMMIT";
const SELECTED_TEMPS_ENV: &str = "OPTIC_SELECTED_TEMPS_DIR";
const WORKSPACE_WRAPPER_ENV: &str = "OPTIC_HAS_WORKSPACE_WRAPPER";
const WRAPPER_ACTIVE_ENV: &str = "OPTIC_RUSTC_WRAPPER_ACTIVE";

pub(crate) struct RustcDriver {
    executable: PathBuf,
    original_global_wrapper: Option<OsString>,
    workspace_wrapper: Option<OsString>,
}

impl RustcDriver {
    pub(crate) fn prepare(toolchain: &Toolchain, workspace_root: &Path) -> Result<Self> {
        if env::var_os(WRAPPER_ACTIVE_ENV).is_some() {
            return Err(Error::CompilerEnvironment {
                message: "a Cargo Optic rustc wrapper is already active".to_owned(),
            });
        }

        require_rustc_dev(toolchain)?;
        let wrappers = effective_wrappers(workspace_root)?;
        let executable = cached_driver(toolchain)?;

        Ok(Self {
            executable,
            original_global_wrapper: wrappers.global,
            workspace_wrapper: wrappers.workspace,
        })
    }

    pub(crate) fn configure(
        &self,
        command: &mut Command,
        analysis_directory: &Path,
        manifest_path: &Path,
        rustc_commit: &str,
    ) {
        command
            .env("RUSTC_WRAPPER", &self.executable)
            .env(WRAPPER_ACTIVE_ENV, "1")
            .env(SELECTED_TEMPS_ENV, analysis_directory)
            .env(MANIFEST_PATH_ENV, manifest_path)
            .env(RUSTC_COMMIT_ENV, rustc_commit)
            .env_remove(DRIVER_INNER_ENV);

        match &self.original_global_wrapper {
            Some(wrapper) => {
                command.env(ORIGINAL_WRAPPER_ENV, wrapper);
            }
            None => {
                command.env_remove(ORIGINAL_WRAPPER_ENV);
            }
        }

        if self.workspace_wrapper.is_some() {
            command.env(WORKSPACE_WRAPPER_ENV, "1");
        } else {
            command.env_remove(WORKSPACE_WRAPPER_ENV);
        }
    }

    pub(crate) fn wrapper_chain(&self) -> Vec<String> {
        let mut wrappers = vec![self.executable.to_string_lossy().into_owned()];

        if let Some(wrapper) = &self.original_global_wrapper {
            wrappers.push(wrapper.to_string_lossy().into_owned());
        }
        if let Some(wrapper) = &self.workspace_wrapper {
            wrappers.push(wrapper.to_string_lossy().into_owned());
        }

        wrappers
    }
}

struct EffectiveWrappers {
    global: Option<OsString>,
    workspace: Option<OsString>,
}

fn effective_wrappers(workspace_root: &Path) -> Result<EffectiveWrappers> {
    let needs_config =
        env::var_os("RUSTC_WRAPPER").is_none() || env::var_os("RUSTC_WORKSPACE_WRAPPER").is_none();
    let config = if needs_config {
        cargo_config(workspace_root)?
    } else {
        Value::Null
    };

    let global = effective_wrapper("RUSTC_WRAPPER", &config, "/build/rustc-wrapper");
    let workspace = effective_wrapper(
        "RUSTC_WORKSPACE_WRAPPER",
        &config,
        "/build/rustc-workspace-wrapper",
    );

    Ok(EffectiveWrappers { global, workspace })
}

fn effective_wrapper(name: &str, config: &Value, pointer: &str) -> Option<OsString> {
    match env::var_os(name) {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
        None => config
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(OsString::from),
    }
}

fn cargo_config(workspace_root: &Path) -> Result<Value> {
    let program = "cargo".to_owned();
    // `CARGO` can name the Cargo binary that built Optic instead of the Cargo binary selected by
    // `RUSTUP_TOOLCHAIN`. The PATH proxy selects the matching nightly for this unstable command.
    let output = Command::new(&program)
        .current_dir(workspace_root)
        .args(["-Z", "unstable-options", "config", "get", "--format=json"])
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

    serde_json::from_slice(&output.stdout).map_err(|source| Error::CompilerEnvironment {
        message: format!("Cargo returned invalid configuration JSON: {source}"),
    })
}

fn require_rustc_dev(toolchain: &Toolchain) -> Result<()> {
    let entries =
        fs::read_dir(&toolchain.rustc_private_lib).map_err(|source| Error::Filesystem {
            operation: "read",
            path: toolchain.rustc_private_lib.clone(),
            source,
        })?;
    let mut has_rustc_middle = false;

    for entry in entries {
        let entry = entry.map_err(|source| Error::Filesystem {
            operation: "read entry in",
            path: toolchain.rustc_private_lib.clone(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("librustc_middle-")
            && (name.ends_with(".rlib") || name.ends_with(".rmeta"))
        {
            has_rustc_middle = true;

            break;
        }
    }

    if !has_rustc_middle {
        return Err(Error::MissingRustcDev {
            path: toolchain.rustc_private_lib.clone(),
            toolchain: env::var("RUSTUP_TOOLCHAIN").unwrap_or_else(|_| "nightly".to_owned()),
        });
    }

    Ok(())
}

fn cached_driver(toolchain: &Toolchain) -> Result<PathBuf> {
    let digest = blake3::hash(DRIVER_SOURCE.as_bytes());
    let directory = cargo_home()?
        .join("optic")
        .join("drivers")
        .join(&toolchain.host)
        .join(&toolchain.commit_hash)
        .join(digest.to_hex().as_str());
    create_private_directory(&directory)?;
    let lock_path = directory.join("build.lock");
    let lock = open_lock(&lock_path)?;
    FileExt::lock_exclusive(&lock).map_err(|source| Error::Filesystem {
        operation: "lock",
        path: lock_path,
        source,
    })?;
    let executable = directory.join(executable_name());

    if executable.is_file() && driver_is_compatible(&executable) {
        return Ok(executable);
    }
    if executable.exists() {
        fs::remove_file(&executable).map_err(|source| Error::Filesystem {
            operation: "remove",
            path: executable.clone(),
            source,
        })?;
    }

    build_driver(toolchain, &directory, &executable)?;
    if !driver_is_compatible(&executable) {
        return Err(Error::CompilerEnvironment {
            message: format!(
                "compiled rustc driver did not report protocol version {PROTOCOL_VERSION}"
            ),
        });
    }

    Ok(executable)
}

fn build_driver(toolchain: &Toolchain, directory: &Path, executable: &Path) -> Result<()> {
    let source = directory.join("driver.rs");
    fs::write(&source, DRIVER_SOURCE).map_err(|source_error| Error::Filesystem {
        operation: "write",
        path: source.clone(),
        source: source_error,
    })?;

    let mut temporary_name = executable_name().to_owned();
    temporary_name.push(".tmp");
    let temporary = directory.join(temporary_name);
    if temporary.is_file() {
        fs::remove_file(&temporary).map_err(|source| Error::Filesystem {
            operation: "remove",
            path: temporary.clone(),
            source,
        })?;
    }
    let program = toolchain.rustc.to_string_lossy().into_owned();
    let output = Command::new(&toolchain.rustc)
        .arg(&source)
        .args(["--crate-name", "optic_rustc_driver", "--edition=2024"])
        .args(["-D", "warnings"])
        .args(["-C", "prefer-dynamic", "-C", "rpath"])
        .arg("-o")
        .arg(&temporary)
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
    fs::rename(&temporary, executable).map_err(|source| Error::Filesystem {
        operation: "publish",
        path: executable.to_owned(),
        source,
    })
}

fn driver_is_compatible(executable: &Path) -> bool {
    Command::new(executable)
        .arg("--optic-version")
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == PROTOCOL_VERSION
        })
}

fn cargo_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("CARGO_HOME")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }

    let home_name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let Some(home) = env::var_os(home_name).filter(|path| !path.is_empty()) else {
        return Err(Error::CompilerEnvironment {
            message: format!("CARGO_HOME and {home_name} are not set"),
        });
    };

    Ok(PathBuf::from(home).join(".cargo"))
}

fn executable_name() -> &'static OsStr {
    if cfg!(windows) {
        OsStr::new("optic-rustc-driver.exe")
    } else {
        OsStr::new("optic-rustc-driver")
    }
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::Filesystem {
        operation: "create",
        path: path.to_owned(),
        source,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, permissions).map_err(|source| Error::Filesystem {
            operation: "set permissions on",
            path: path.to_owned(),
            source,
        })?;
    }

    Ok(())
}

fn open_lock(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|source| Error::Filesystem {
            operation: "open",
            path: path.to_owned(),
            source,
        })
}
