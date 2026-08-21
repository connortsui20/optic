//! Provisions the exact-version rustc driver and preserves Cargo wrapper composition.
//!
//! The driver is a small, dependency-free program embedded in `cargo-ir`. Cargo's selected compiler
//! builds it once for each rustc commit, sysroot, and source revision. Cargo then uses it as the
//! outer global wrapper without changing workspace artifact hashes.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::toolchain::CargoContext;
use crate::{Error, Result, Toolchain};
use fs2::FileExt;

const DRIVER_SOURCE: &str = include_str!("../driver/main.rs");
const PROTOCOL_VERSION: &str = "3";

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
    pub(crate) fn prepare(toolchain: &Toolchain, cargo: &CargoContext) -> Result<Self> {
        if env::var_os(WRAPPER_ACTIVE_ENV).is_some() {
            return Err(Error::CompilerEnvironment {
                message: "a Cargo Optic rustc wrapper is already active".to_owned(),
            });
        }

        require_rustc_dev(toolchain)?;
        let executable = cached_driver(toolchain, cargo)?;

        Ok(Self {
            executable,
            original_global_wrapper: cargo.global_wrapper().map(OsStr::to_owned),
            workspace_wrapper: cargo.workspace_wrapper().map(OsStr::to_owned),
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
            .env_remove("RUSTC_BOOTSTRAP")
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

fn require_rustc_dev(toolchain: &Toolchain) -> Result<()> {
    let missing = || Error::MissingRustcDev {
        release: toolchain.release.clone(),
        commit_hash: toolchain.commit_hash.clone(),
        path: toolchain.rustc_private_lib.clone(),
        install_command: toolchain
            .rustup_toolchain
            .as_ref()
            .map(|toolchain| format!("rustup component add --toolchain {toolchain} rustc-dev")),
    };
    let entries = match fs::read_dir(&toolchain.rustc_private_lib) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Err(missing()),
        Err(source) => {
            return Err(Error::Filesystem {
                operation: "read",
                path: toolchain.rustc_private_lib.clone(),
                source,
            });
        }
    };
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
        return Err(missing());
    }

    Ok(())
}

fn cached_driver(toolchain: &Toolchain, cargo: &CargoContext) -> Result<PathBuf> {
    let directory = driver_cache_directory(&cargo_home()?, toolchain);
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

    build_driver(toolchain, cargo, &directory, &executable)?;
    if !driver_is_compatible(&executable) {
        return Err(Error::CompilerEnvironment {
            message: format!(
                "compiled rustc driver did not report protocol version {PROTOCOL_VERSION}"
            ),
        });
    }

    Ok(executable)
}

fn driver_cache_directory(cargo_home: &Path, toolchain: &Toolchain) -> PathBuf {
    let source_digest = blake3::hash(DRIVER_SOURCE.as_bytes());
    let sysroot_digest = blake3::hash(toolchain.sysroot.as_os_str().as_encoded_bytes());

    cargo_home
        .join("optic")
        .join("drivers")
        .join(&toolchain.host)
        .join(&toolchain.commit_hash)
        .join(sysroot_digest.to_hex().as_str())
        .join(source_digest.to_hex().as_str())
        .join(PROTOCOL_VERSION)
}

fn build_driver(
    toolchain: &Toolchain,
    cargo: &CargoContext,
    directory: &Path,
    executable: &Path,
) -> Result<()> {
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
        .current_dir(cargo.workspace_root())
        .arg(&source)
        .args(["--crate-name", "optic_rustc_driver", "--edition=2024"])
        .args(["-D", "warnings"])
        .args(["-C", "prefer-dynamic", "-C", "rpath"])
        .arg("-o")
        .arg(&temporary)
        .env("RUSTC_BOOTSTRAP", "1")
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

#[cfg(test)]
mod tests {
    use super::driver_cache_directory;
    use crate::Toolchain;
    use std::path::{Path, PathBuf};

    #[test]
    fn separates_drivers_for_distinct_canonical_sysroots() {
        let first = toolchain("/rustup/toolchains/first");
        let second = toolchain("/rustup/toolchains/second");

        assert_ne!(
            driver_cache_directory(Path::new("/cargo-home"), &first),
            driver_cache_directory(Path::new("/cargo-home"), &second)
        );
    }

    fn toolchain(sysroot: &str) -> Toolchain {
        Toolchain {
            rustc: PathBuf::from("rustc"),
            release: "1.97.1".to_owned(),
            commit_hash: "abc123".to_owned(),
            host: "test-host".to_owned(),
            llvm_version: "22.1.6".to_owned(),
            sysroot: PathBuf::from(sysroot),
            rustc_private_lib: PathBuf::from(sysroot).join("lib"),
            llvm_dis: PathBuf::from(sysroot).join("llvm-dis"),
            rustup_toolchain: None,
        }
    }
}
