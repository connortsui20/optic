//! Exercises the supported compiler environment through real Cargo processes.

#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use optic_compiler::BuildRequest;
use optic_compiler::CargoTarget;
use optic_compiler::collect_build;
use optic_compiler::discover_workspace;

fn write_package(directory: &Path) {
    fs::create_dir(directory.join("src")).unwrap();
    fs::create_dir(directory.join(".cargo")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"selection_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(directory.join("src/lib.rs"), "pub fn collected() {}\n").unwrap();
}

fn child_command(directory: &Path, scenario: &str) -> Command {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--exact", "collect_in_child", "--nocapture"])
        .env("OPTIC_TEST_WORKSPACE", directory)
        .env("OPTIC_TEST_SCENARIO", scenario)
        .env_remove("RUSTC")
        .env_remove("CARGO_BUILD_RUSTC")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER");

    command
}

#[test]
fn collects_with_the_default_rustc() {
    let temporary = tempfile::tempdir().unwrap();
    write_package(temporary.path());
    fs::write(temporary.path().join(".cargo/config.toml"), "").unwrap();
    let output = child_command(temporary.path(), "success")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", diagnostics(&output));
}

#[test]
fn collects_a_warm_target_again() {
    let temporary = tempfile::tempdir().unwrap();
    write_package(temporary.path());
    fs::write(temporary.path().join(".cargo/config.toml"), "").unwrap();
    let output = child_command(temporary.path(), "warm")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", diagnostics(&output));
}

#[test]
fn disables_a_configured_wrapper_with_a_warning() {
    let temporary = tempfile::tempdir().unwrap();
    write_package(temporary.path());
    let wrapper = temporary.path().join("wrapper");
    fs::write(
        &wrapper,
        "#!/bin/sh\ntouch \"$OPTIC_TEST_WORKSPACE/wrapper-ran\"\nexec \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        temporary.path().join(".cargo/config.toml"),
        "[build]\nrustc-wrapper = \"./wrapper\"\n",
    )
    .unwrap();

    let output = child_command(temporary.path(), "success").output().unwrap();
    let diagnostics = diagnostics(&output);

    assert!(output.status.success(), "{diagnostics}");
    assert!(!temporary.path().join("wrapper-ran").exists());
    assert!(diagnostics.contains(
        "warning: Cargo Optic does not support configured rustc wrappers; disabling them for this capture"
    ));
    assert!(
        diagnostics.contains(
            "warning: the captured compiler output can differ from a normal wrapped build"
        )
    );
}

#[test]
fn rejects_a_configured_compiler() {
    let temporary = tempfile::tempdir().unwrap();
    write_package(temporary.path());
    fs::write(
        temporary.path().join(".cargo/config.toml"),
        "[build]\nrustc = \"rustc\"\n",
    )
    .unwrap();
    let output = child_command(temporary.path(), "compiler-error")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", diagnostics(&output));
}

#[test]
fn rejects_an_environment_compiler() {
    let temporary = tempfile::tempdir().unwrap();
    write_package(temporary.path());
    fs::write(temporary.path().join(".cargo/config.toml"), "").unwrap();
    let output = child_command(temporary.path(), "compiler-error")
        .env("RUSTC", "rustc")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", diagnostics(&output));
}

#[test]
fn collect_in_child() {
    let Some(directory) = env::var_os("OPTIC_TEST_WORKSPACE") else {
        return;
    };
    let workspace = discover_workspace(Path::new(&directory)).unwrap();
    let request = BuildRequest::new("selection_fixture", CargoTarget::Library, "release").unwrap();
    let result = collect_build(&workspace, &request);

    match env::var("OPTIC_TEST_SCENARIO").unwrap().as_str() {
        "success" => {
            result.expect("collection with the default compiler must succeed");
        }
        "warm" => {
            result.expect("the first collection must succeed");
            collect_build(&workspace, &request).expect("the warm collection must run rustc again");
        }
        "compiler-error" => {
            let Err(error) = result else {
                panic!("custom compiler selection must fail");
            };
            assert!(
                error
                    .to_string()
                    .contains("custom rustc selection is not supported")
            );
        }
        scenario => panic!("unknown test scenario {scenario}"),
    }
}

fn diagnostics(output: &std::process::Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}
