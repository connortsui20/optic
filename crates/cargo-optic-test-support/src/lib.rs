//! Isolates Cargo fixtures and child processes for product integration tests.
//!
//! [`TestWorkspace`] owns the copied files and private build directories. Tests construct commands
//! and assert product behavior. Only child commands receive the isolated environment.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

/// A copied fixture whose files remain alive until the test finishes.
pub struct TestWorkspace {
    root: TempDir,
    cargo: PathBuf,
    toolchain: PathBuf,
    rustup_home: PathBuf,
    path: OsString,
}

impl TestWorkspace {
    /// Copies a bundled fixture name or an absolute fixture directory into a private workspace.
    ///
    /// Resolves the active rustup toolchain before isolation. Panics if the fixture or installed
    /// toolchain cannot be read. Fixtures must contain only ordinary files and directories.
    pub fn new(fixture: impl AsRef<Path>) -> Self {
        let mut resolve = Command::new("rustup");
        resolve.args(["which", "cargo"]);
        let output = run(&mut resolve);
        assert_success(&resolve, &output);
        let cargo = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        let toolchain = cargo.parent().unwrap().parent().unwrap().to_owned();
        let rustup_home = env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| toolchain.parent().unwrap().parent().unwrap().to_owned());
        let rustup_home = fs::canonicalize(rustup_home).unwrap();

        // The real toolchain precedes system tools. Developer wrappers and Cargo configuration do
        // not enter the child through PATH or HOME.
        let path = env::join_paths([
            toolchain.join("bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/sbin"),
        ])
        .unwrap();
        let root = tempfile::Builder::new()
            .prefix("optic-test-")
            .tempdir()
            .unwrap();

        for directory in [
            "cargo-home",
            "target",
            "build",
            "temp",
            "observations",
            "workspace",
        ] {
            fs::create_dir(root.path().join(directory)).unwrap();
        }

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(fixture);
        copy_fixture(&fixture, &root.path().join("workspace"));

        Self {
            root,
            cargo,
            toolchain,
            rustup_home,
            path,
        }
    }

    /// Returns the canonical workspace directory that contains the ordinary Optic store.
    pub fn workspace(&self) -> PathBuf {
        fs::canonicalize(self.root.path().join("workspace")).unwrap()
    }

    /// Returns the isolated Cargo home, including the driver cache.
    pub fn cargo_home(&self) -> PathBuf {
        self.root.path().join("cargo-home")
    }

    /// Returns the Cargo artifact directory.
    pub fn target(&self) -> PathBuf {
        self.root.path().join("target")
    }

    /// Returns the Cargo intermediate build directory.
    pub fn build(&self) -> PathBuf {
        self.root.path().join("build")
    }

    /// Returns the temporary directory for child processes.
    pub fn temp(&self) -> PathBuf {
        self.root.path().join("temp")
    }

    /// Returns the directory for test observations outside Cargo's source scan.
    pub fn observations(&self) -> PathBuf {
        self.root.path().join("observations")
    }

    /// Clears the child environment and applies the fixture paths and installed toolchain.
    ///
    /// Tests must apply scenario-specific environment changes after this call. The parent process
    /// environment remains unchanged. Child Cargo commands operate offline without automatic rustup
    /// installation, developer Cargo configuration, compiler overrides, or inherited wrappers.
    pub fn apply(&self, command: &mut Command) {
        command
            .env_clear()
            .current_dir(self.workspace())
            .env("PATH", &self.path)
            .env("HOME", self.root.path())
            .env("CARGO", &self.cargo)
            .env("CARGO_HOME", self.cargo_home())
            .env("CARGO_TARGET_DIR", self.target())
            .env("CARGO_BUILD_BUILD_DIR", self.build())
            .env("CARGO_NET_OFFLINE", "true")
            .env("CARGO_TERM_COLOR", "never")
            .env("RUSTUP_HOME", &self.rustup_home)
            .env("RUSTUP_TOOLCHAIN", &self.toolchain)
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("TMPDIR", self.temp())
            .env("TMP", self.temp())
            .env("TEMP", self.temp());

        for name in ["SYSTEMROOT", "SDKROOT", "DEVELOPER_DIR"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }

        // Cargo-built test executables can link to shared libraries in the compiler sysroot.
        // Reconstruct this path instead of inheriting arbitrary loader search directories.
        command.env("LD_LIBRARY_PATH", self.toolchain.join("lib"));
        command.env("DYLD_LIBRARY_PATH", self.toolchain.join("lib"));
    }
}

fn copy_fixture(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().unwrap();

        if kind.is_dir() {
            fs::create_dir(&target).unwrap();
            copy_fixture(&entry.path(), &target);
        } else {
            assert!(
                kind.is_file(),
                "fixture entries must be ordinary files or directories"
            );
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Runs a command and retains its output even when the command fails.
#[track_caller]
pub fn run(command: &mut Command) -> Output {
    command.output().unwrap_or_else(|error| {
        panic!("cannot start {}: {error}", command_context(command));
    })
}

/// Requires successful completion and reports the isolated command context on failure.
#[track_caller]
pub fn assert_success(command: &Command, output: &Output) {
    assert!(output.status.success(), "{}", diagnostics(command, output));
}

/// Describes command failure without exposing unrelated inherited environment values.
pub fn diagnostics(command: &Command, output: &Output) -> String {
    format!(
        "{}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        command_context(command),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn command_context(command: &Command) -> String {
    let toolchain = command
        .get_envs()
        .find(|(name, _)| *name == "RUSTUP_TOOLCHAIN");

    format!(
        "command: {:?} {:?}\ncwd: {:?}\ntoolchain: {:?}",
        command.get_program(),
        command.get_args().collect::<Vec<_>>(),
        command.get_current_dir(),
        toolchain.and_then(|(_, value)| value),
    )
}
