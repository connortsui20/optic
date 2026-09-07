//! Exercises private compiler collection and provisioning with self-contained unit fixtures.
//!
//! Child environments own their Cargo home and temporary paths. The parent never mutates process
//! environment variables, and the provisioning counter exists only in compiler unit-test builds.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use optic_records::CaptureId;
use optic_records::CompilerIdentity;

use crate::BuildRequest;
use crate::CargoTarget;
use crate::Error;
use crate::discover_workspace;
use crate::driver::RustcDriver;
use crate::prepare_build;
use crate::toolchain::CompilerContext;

/// Supplies one fresh selected artifact whose feature order still needs normalization.
#[track_caller]
pub(crate) fn artifact() -> cargo_metadata::Artifact {
    serde_json::from_value(serde_json::json!({
        "package_id": "path+file:///workspace#fixture@0.1.0",
        "manifest_path": "/workspace/Cargo.toml",
        "target": {
            "kind": ["lib"],
            "crate_types": ["lib"],
            "name": "fixture",
            "src_path": "/workspace/src/lib.rs",
            "edition": "2024",
            "doc": true,
            "doctest": true,
            "test": true
        },
        "profile": {
            "opt_level": "2",
            "debuginfo": 2,
            "debug_assertions": false,
            "overflow_checks": true,
            "test": false
        },
        "features": ["beta", "alpha"],
        "filenames": ["/target/libfixture.rlib"],
        "executable": null,
        "fresh": true
    }))
    .unwrap()
}

/// Owns an inline package and isolated child environment for private compiler tests.
struct PrivateFixture {
    directory: tempfile::TempDir,
    /// The canonical installed sysroot whose real Cargo and rustc executables enter the child.
    toolchain: PathBuf,
}

impl PrivateFixture {
    #[track_caller]
    fn new() -> Self {
        let output = run(Command::new("rustc").args(["--print", "sysroot"]));
        let toolchain = fs::canonicalize(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
        let directory = tempfile::Builder::new()
            .prefix("optic-compiler-unit-")
            .tempdir_in(fs::canonicalize(env::temp_dir()).unwrap())
            .unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("package/src")).unwrap();
        fs::create_dir(root.join("cargo-home")).unwrap();
        fs::create_dir(root.join("temp")).unwrap();
        fs::write(
            root.join("package/Cargo.toml"),
            "[package]\n\
             name = 'compiler_fixture'\n\
             version = '0.0.0'\n\
             edition = '2024'\n\
             \n\
             [workspace]\n",
        )
        .unwrap();
        fs::write(
            root.join("package/src/lib.rs"),
            "#[unsafe(no_mangle)]\npub fn retained(value: u64) -> u64 { value.wrapping_add(1) }\n",
        )
        .unwrap();

        Self {
            directory,
            toolchain,
        }
    }

    #[track_caller]
    fn child(&self, test: &str) {
        let root = self.directory.path();
        let path = env::join_paths([
            self.toolchain.join("bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/sbin"),
        ])
        .unwrap();
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .env_clear()
            .args(["--exact", test, "--nocapture"])
            .current_dir(root.join("package"))
            .env("PATH", path)
            .env("HOME", root)
            .env("CARGO", self.toolchain.join("bin/cargo"))
            .env("CARGO_HOME", root.join("cargo-home"))
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("CARGO_NET_OFFLINE", "true")
            .env("CARGO_TERM_COLOR", "never")
            .env("TMPDIR", root.join("temp"))
            .env("TMP", root.join("temp"))
            .env("TEMP", root.join("temp"))
            .env("OPTIC_TEST_CHILD", "1")
            .env("OPTIC_TEST_DRIVER_BUILDS", root.join("driver-builds"));

        // Reconstruct compiler-library lookup paths without inheriting arbitrary loader settings.
        command.env("LD_LIBRARY_PATH", self.toolchain.join("lib"));
        command.env("DYLD_LIBRARY_PATH", self.toolchain.join("lib"));
        for name in ["SDKROOT", "DEVELOPER_DIR"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }

        run(&mut command);
    }
}

#[track_caller]
fn run(command: &mut Command) -> std::process::Output {
    let output = command.output().unwrap_or_else(|error| {
        panic!("cannot start {:?}: {error}", command.get_program());
    });
    assert!(
        output.status.success(),
        "{:?} {:?}: {}\nstdout:\n{}\nstderr:\n{}",
        command.get_program(),
        command.get_args().collect::<Vec<_>>(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    output
}

#[test]
fn driver_cache_reuses_actual_compilation_across_processes() {
    let fixture = PrivateFixture::new();
    let count = fixture.directory.path().join("driver-builds");
    fixture.child("tests::driver_cache_child");
    assert_eq!(fs::read_to_string(&count).unwrap(), "build\n");
    fixture.child("tests::driver_cache_child");
    assert_eq!(fs::read_to_string(&count).unwrap(), "build\n");
}

#[test]
fn driver_cache_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let compiler = CompilerContext::discover(&workspace).unwrap();
    RustcDriver::provision(&workspace, compiler.identity()).unwrap();
}

#[test]
fn failed_driver_build_reports_components_without_publishing_a_cache_entry() {
    let fixture = PrivateFixture::new();
    fixture.child("tests::failed_driver_build_child");

    assert_eq!(
        fs::read_to_string(fixture.directory.path().join("driver-builds")).unwrap(),
        "build\n"
    );
    assert_eq!(
        fs::read_dir(fixture.directory.path().join("cargo-home/optic/drivers"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn failed_driver_build_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let compiler = CompilerContext::discover(&workspace).unwrap();
    let executable = PathBuf::from(env::var_os("TMPDIR").unwrap()).join("failed-rustc");
    fs::write(
        &executable,
        r#"#!/bin/sh
set -eu
previous=
for argument in "$@"; do
    if [ "$previous" = -o ]; then
        printf '%s\n' incomplete-driver > "$argument"
    fi
    previous=$argument
done
printf '%s\n' 'error[E0463]: rustc_driver is unavailable in this sysroot' >&2
exit 23
"#,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let identity = compiler.identity();
    let failing = CompilerIdentity::new(
        executable.clone(),
        identity.release(),
        identity.commit_hash(),
        identity.host(),
        identity.sysroot().to_owned(),
    )
    .unwrap();

    let Err(Error::ProcessFailed {
        program,
        status,
        diagnostics,
    }) = RustcDriver::provision(&workspace, &failing)
    else {
        panic!("the compiler shim must return its driver-build failure");
    };
    assert_eq!(program, executable);
    assert_eq!(status, "exit status: 23");
    let diagnostics = diagnostics.expect("driver-build failures retain compiler diagnostics");
    assert!(
        diagnostics.contains("error[E0463]: rustc_driver is unavailable in this sysroot"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("rustup component add rustc-dev llvm-tools"),
        "{diagnostics}"
    );
}

#[test]
fn expected_bitcode_failures_do_not_return_partial_collections() {
    PrivateFixture::new().child("tests::expected_bitcode_failure_child");
}

#[test]
fn expected_bitcode_failure_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new("compiler_fixture", CargoTarget::Library, "release").unwrap();
    let (_, compiler, analysis, _, temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    let marker = format!(
        "{}\"{}\"",
        crate::protocol::MARKER_PREFIX,
        analysis.token().as_str()
    );
    let private_manifest = temporary.path().join("manifest");
    let raw = crate::manifest::read_manifest(&private_manifest, &marker).unwrap();
    let bitcode = raw
        .modules
        .first()
        .expect("the exported function produces an optimized module")
        .path
        .clone();

    for bytes in [
        Some(&b""[..]),                // Empty expected bitcode.
        Some(&b"invalid bitcode"[..]), // Corrupt expected bitcode.
        None,                          // Missing expected bitcode.
    ] {
        if let Some(bytes) = bytes {
            fs::write(&bitcode, bytes).unwrap();
        } else {
            fs::remove_file(&bitcode).unwrap();
        }

        let raw = crate::manifest::read_manifest(&private_manifest, &marker).unwrap();
        let error = match crate::artifacts::collect(raw, &compiler, temporary.path()) {
            Ok(_) => panic!("a missing or malformed expected module must fail collection"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(bitcode.to_str().unwrap()),
            "{error}"
        );
    }
}
