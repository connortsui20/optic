//! Provisions the standalone driver for one exact compiler and embedded recipe.
//!
//! Entries live in Cargo home and retain a stable executable path across processes. Provisioning
//! requires one writer per Cargo home. A present invalid entry is an error, not a cache miss.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use optic_records::CompilerIdentity;

use crate::Error;
use crate::Workspace;
use crate::digest::digest;
use crate::protocol;

/// The complete embedded source set, shared by cache hashing and standalone compilation.
const SOURCES: [(&str, &str); 5] = [
    ("main.rs", include_str!("../rustc-driver/main.rs")),
    ("analysis.rs", include_str!("../rustc-driver/analysis.rs")),
    ("manifest.rs", include_str!("../rustc-driver/manifest.rs")),
    ("protocol.rs", include_str!("../rustc-driver/protocol.rs")),
    ("wrapper.rs", include_str!("../rustc-driver/wrapper.rs")),
];

/// Identifies the source layout, cache header, and scoped bootstrap compilation recipe.
const RECIPE_REVISION: &[u8] = b"optic-driver-recipe-1";
/// Fixed build options in command order. The resolved compiler supplies the final sysroot value.
const BUILD_OPTIONS: [&str; 8] = [
    "--crate-name",
    "optic_rustc_driver",
    "--edition=2024",
    "-C",
    "prefer-dynamic",
    "-C",
    "rpath",
    "--sysroot",
];

/// A validated driver cache entry for the selected compiler.
pub(crate) struct RustcDriver {
    executable: PathBuf,
    key: String,
}

impl RustcDriver {
    pub(crate) fn provision(
        workspace: &Workspace,
        compiler: &CompilerIdentity,
    ) -> Result<Self, Error> {
        let key = driver_key(
            compiler,
            &SOURCES,
            &BUILD_OPTIONS,
            RECIPE_REVISION,
            protocol::PROTOCOL_VERSION,
        );
        let root = cargo_home(workspace)?.join("optic/drivers");
        fs::create_dir_all(&root).map_err(|source| Error::Filesystem {
            operation: "create driver cache",
            path: root.clone(),
            source,
        })?;
        let entry = root.join(&key);

        match fs::symlink_metadata(&entry) {
            Ok(_) => {
                validate_entry(&entry, &key)?;

                return Ok(Self {
                    executable: entry.join("optic-rustc-driver"),
                    key,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::Filesystem {
                    operation: "inspect driver cache entry",
                    path: entry,
                    source,
                });
            }
        }

        let temporary = tempfile::Builder::new()
            .prefix(".driver-")
            .tempdir_in(&root)
            .map_err(|source| Error::Filesystem {
                operation: "create driver build directory in",
                path: root,
                source,
            })?;

        for (name, contents) in SOURCES {
            let path = temporary.path().join(name);
            fs::write(&path, contents).map_err(|source| Error::Filesystem {
                operation: "write embedded driver source",
                path,
                source,
            })?;
        }

        let executable = temporary.path().join("optic-rustc-driver");
        build_driver(
            workspace,
            compiler,
            &temporary.path().join("main.rs"),
            &executable,
            &key,
        )?;
        let header = temporary.path().join("identity");
        fs::write(&header, format!("optic-driver-1\n{key}\n")).map_err(|source| {
            Error::Filesystem {
                operation: "write driver identity",
                path: header,
                source,
            }
        })?;
        validate_entry(temporary.path(), &key)?;
        fs::rename(temporary.path(), &entry).map_err(|source| Error::Filesystem {
            operation: "publish driver cache entry",
            path: entry.clone(),
            source,
        })?;

        Ok(Self {
            executable: entry.join("optic-rustc-driver"),
            key,
        })
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// Installs the stable wrapper and attempt-local output paths without granting bootstrap.
    pub(crate) fn configure(&self, command: &mut Command, marker: &str, manifest_path: &Path) {
        command
            .env("RUSTC_WRAPPER", &self.executable)
            .env("RUSTC_WORKSPACE_WRAPPER", "")
            .env("CARGO_BUILD_RUSTC_WRAPPER", "")
            .env("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "")
            .env(protocol::SELECTED_TARGET_MARKER_ENV, marker)
            .env(protocol::MANIFEST_PATH_ENV, manifest_path)
            .env_remove(protocol::DRIVER_INNER_ENV)
            .env_remove("RUSTC_BOOTSTRAP");
    }
}

pub(crate) fn cargo_home(workspace: &Workspace) -> Result<PathBuf, Error> {
    let home = env::var_os("CARGO_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .ok_or_else(|| Error::CompilerEnvironment {
            message: "Cargo home requires CARGO_HOME or HOME, got neither".to_owned(),
        })?;

    Ok(workspace.invocation_directory().join(home))
}

fn driver_key(
    compiler: &CompilerIdentity,
    sources: &[(&str, &str)],
    options: &[&str],
    recipe_revision: &[u8],
    protocol_version: u32,
) -> String {
    let version = protocol_version.to_le_bytes();
    let mut fields = vec![
        recipe_revision,
        compiler.rustc().as_os_str().as_encoded_bytes(),
        compiler.release().as_bytes(),
        compiler.commit_hash().as_bytes(),
        compiler.host().as_bytes(),
        compiler.sysroot().as_os_str().as_encoded_bytes(),
        &version,
        b"RUSTC_BOOTSTRAP=optic_rustc_driver",
        b"OPTIC_DRIVER_KEY=recipe-digest",
        b"-o=optic-rustc-driver",
    ];

    for (name, source) in sources {
        fields.push(name.as_bytes());
        fields.push(source.as_bytes());
    }

    fields.extend(options.iter().map(|option| option.as_bytes()));

    digest(&fields)
}

fn validate_entry(entry: &Path, key: &str) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;

    for (path, directory) in [
        (entry.to_owned(), true),
        (entry.join("identity"), false),
        (entry.join("optic-rustc-driver"), false),
    ] {
        let metadata = fs::symlink_metadata(&path).map_err(|source| Error::Filesystem {
            operation: "validate driver cache entry",
            path: path.clone(),
            source,
        })?;
        let correct_kind = if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        };
        if metadata.file_type().is_symlink() || !correct_kind {
            return Err(Error::CompilerEnvironment {
                message: format!(
                    "driver cache requires regular files and directories, got {}",
                    path.display()
                ),
            });
        }
        if path
            .file_name()
            .is_some_and(|name| name == "optic-rustc-driver")
            && (metadata.len() == 0 || metadata.permissions().mode() & 0o111 == 0)
        {
            return Err(Error::CompilerEnvironment {
                message: format!(
                    "driver cache requires a nonempty executable, got {}",
                    path.display()
                ),
            });
        }
    }

    let path = entry.join("identity");
    let expected = format!("optic-driver-1\n{key}\n");
    let metadata = fs::metadata(&path).map_err(|source| Error::Filesystem {
        operation: "inspect driver identity",
        path: path.clone(),
        source,
    })?;
    if metadata.len() != expected.len() as u64
        || fs::read(&path).map_err(|source| Error::Filesystem {
            operation: "read driver identity",
            path: path.clone(),
            source,
        })? != expected.as_bytes()
    {
        return Err(Error::CompilerEnvironment {
            message: format!(
                "driver identity must match the current recipe, got {}",
                path.display()
            ),
        });
    }

    let executable = entry.join("optic-rustc-driver");
    let output = Command::new(&executable)
        .arg(protocol::DRIVER_KEY_ARGUMENT)
        .output()
        .map_err(|source| Error::StartProcess {
            program: executable.clone(),
            source,
        })?;
    if !output.status.success() || output.stdout != format!("{key}\n").as_bytes() {
        return Err(Error::CompilerEnvironment {
            message: format!(
                "cached executable must report the prepared driver key, got {}",
                executable.display()
            ),
        });
    }

    Ok(())
}

fn build_driver(
    workspace: &Workspace,
    compiler: &CompilerIdentity,
    source: &Path,
    executable: &Path,
    key: &str,
) -> Result<(), Error> {
    #[cfg(test)]
    if let Some(path) = env::var_os("OPTIC_TEST_DRIVER_BUILDS") {
        use std::io::Write;
        writeln!(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("the test supplies a writable counter"),
            "build"
        )
        .expect("the test counter accepts the build observation");
    }

    let output = Command::new(compiler.rustc())
        .current_dir(workspace.invocation_directory())
        .arg(source)
        .args(BUILD_OPTIONS)
        .arg(compiler.sysroot())
        .arg("-o")
        .arg(executable)
        .env("RUSTC_BOOTSTRAP", "optic_rustc_driver")
        .env("OPTIC_DRIVER_KEY", key)
        .output()
        .map_err(|source| Error::StartProcess {
            program: compiler.rustc().to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: compiler.rustc().to_owned(),
            status: output.status.to_string(),
            diagnostics: Some(format!(
                "{}\nDriver compilation requires rustc-dev and llvm-tools for this exact toolchain. Install those components with rustup component add rustc-dev llvm-tools.",
                String::from_utf8_lossy(&output.stderr)
            )),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiler() -> CompilerIdentity {
        CompilerIdentity::new(
            "/toolchain/bin/rustc".into(),
            "1.98.1",
            "commit",
            "host",
            "/toolchain".into(),
        )
        .unwrap()
    }

    #[test]
    fn key_covers_every_source_and_build_recipe_input() {
        let compiler = compiler();
        let original = driver_key(
            &compiler,
            &SOURCES,
            &BUILD_OPTIONS,
            RECIPE_REVISION,
            protocol::PROTOCOL_VERSION,
        );
        for index in 0..SOURCES.len() {
            let mut sources = SOURCES;
            sources[index].1 = "changed source";
            assert_ne!(
                original,
                driver_key(
                    &compiler,
                    &sources,
                    &BUILD_OPTIONS,
                    RECIPE_REVISION,
                    protocol::PROTOCOL_VERSION
                )
            );
        }
        assert_ne!(
            original,
            driver_key(
                &compiler,
                &SOURCES,
                &["different flags"],
                RECIPE_REVISION,
                protocol::PROTOCOL_VERSION
            )
        );
        assert_ne!(
            original,
            driver_key(
                &compiler,
                &SOURCES,
                &BUILD_OPTIONS,
                b"new recipe",
                protocol::PROTOCOL_VERSION
            )
        );
        assert_ne!(
            original,
            driver_key(
                &compiler,
                &SOURCES,
                &BUILD_OPTIONS,
                RECIPE_REVISION,
                protocol::PROTOCOL_VERSION + 1
            )
        );
    }

    #[test]
    fn key_covers_exact_compiler_identity() {
        let original = driver_key(
            &compiler(),
            &SOURCES,
            &BUILD_OPTIONS,
            RECIPE_REVISION,
            protocol::PROTOCOL_VERSION,
        );
        let cases = [
            ("/other/bin/rustc", "1.98.1", "commit", "host", "/toolchain"), // Executable.
            (
                "/toolchain/bin/rustc",
                "other",
                "commit",
                "host",
                "/toolchain",
            ), // Release.
            (
                "/toolchain/bin/rustc",
                "1.98.1",
                "other",
                "host",
                "/toolchain",
            ), // Commit.
            (
                "/toolchain/bin/rustc",
                "1.98.1",
                "commit",
                "other",
                "/toolchain",
            ), // Host.
            ("/toolchain/bin/rustc", "1.98.1", "commit", "host", "/other"), // Sysroot.
        ];

        for (rustc, release, commit, host, sysroot) in cases {
            let changed =
                CompilerIdentity::new(rustc.into(), release, commit, host, sysroot.into()).unwrap();
            assert_ne!(
                original,
                driver_key(
                    &changed,
                    &SOURCES,
                    &BUILD_OPTIONS,
                    RECIPE_REVISION,
                    protocol::PROTOCOL_VERSION
                )
            );
        }
    }

    #[test]
    fn present_invalid_driver_entries_are_errors() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let entry = temporary.path();
        fs::write(entry.join("identity"), "optic-driver-1\nkey\n").unwrap();
        fs::write(
            entry.join("optic-rustc-driver"),
            "#!/bin/sh\nprintf '%s\\n' key\n",
        )
        .unwrap();
        fs::set_permissions(
            entry.join("optic-rustc-driver"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        validate_entry(entry, "key").unwrap();
        assert!(validate_entry(entry, "wrong key").is_err());
        fs::set_permissions(
            entry.join("optic-rustc-driver"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(validate_entry(entry, "key").is_err());
        fs::remove_file(entry.join("optic-rustc-driver")).unwrap();
        assert!(validate_entry(entry, "key").is_err());
    }
}
