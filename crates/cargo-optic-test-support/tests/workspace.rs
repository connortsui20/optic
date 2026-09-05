//! Exercises fixture isolation with the installed Cargo and compiler.
//!
//! The child builds offline with a private Cargo home and ignores hostile command overrides.

use std::fs;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::run;

#[test]
fn builds_offline_in_private_directories() {
    let workspace = TestWorkspace::new("capture");
    let lockfile = fs::read(workspace.workspace().join("Cargo.lock")).unwrap();
    let mut command = Command::new("cargo");
    command
        .env("RUSTC", "missing-test-compiler")
        .env("RUSTC_WRAPPER", "missing-test-wrapper")
        .env("RUSTFLAGS", "--invalid-test-flag")
        .env("CARGO_ENCODED_RUSTFLAGS", "--invalid-test-flag");
    workspace.apply(&mut command);
    command.args(["build", "--locked", "--all-features"]);

    let output = run(&mut command);
    assert_success(&command, &output);
    assert!(workspace.target().join("debug/generic").is_file());
    assert!(workspace.target().join("debug/feature-gated").is_file());
    assert!(workspace.build().join("debug/deps").is_dir());
    assert!(!workspace.workspace().join("target").exists());
    assert_eq!(
        lockfile,
        fs::read(workspace.workspace().join("Cargo.lock")).unwrap()
    );

    let mut command = Command::new("rustc");
    workspace.apply(&mut command);
    command.args(["--print", "sysroot"]);
    let output = run(&mut command);
    assert_success(&command, &output);
    assert!(std::path::Path::new(String::from_utf8(output.stdout).unwrap().trim()).is_absolute());
}
