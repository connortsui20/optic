//! Protects user-visible command behavior across process boundaries.
//!
//! These tests keep full command integration separate from lower-level contracts.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

fn run<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command(directory)
        .args(arguments)
        .output()
        .expect("the Cargo Optic binary can run")
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-optic"));
    command.current_dir(directory).arg("optic");

    command
}

fn copy_fixture(directory: &Path) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/capture");
    fs::copy(fixture.join("Cargo.toml"), directory.join("Cargo.toml"))
        .expect("the fixture manifest can be copied");
    fs::create_dir(directory.join("src")).expect("the fixture source directory can be created");
    fs::copy(fixture.join("src/lib.rs"), directory.join("src/lib.rs"))
        .expect("the fixture source can be copied");
    fs::copy(
        fixture.join("src/feature_gated.rs"),
        directory.join("src/feature_gated.rs"),
    )
    .expect("the feature-gated fixture source can be copied");
}

#[track_caller]
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn exposes_the_cargo_subcommand_entry_point() {
    let temporary = tempfile::tempdir().expect("the test directory can be created");

    let output = command(temporary.path())
        .arg("--help")
        .output()
        .expect("the Cargo Optic binary can run");
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).expect("help output is UTF-8");

    assert!(stdout.contains("Usage: cargo optic <COMMAND>"));
}

#[test]
fn reports_an_empty_capture_history() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());

    let output = run(temporary.path(), ["list-captures"]);
    assert_success(&output);

    assert_eq!(output.stdout, b"No captures.\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn captures_a_fixture_target_and_lists_the_completed_record() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());

    let captured = run(
        temporary.path(),
        ["capture", "-p", "capture_fixture", "--lib", "--release"],
    );
    assert_success(&captured);
    let captured_text = String::from_utf8(captured.stdout).expect("capture output is UTF-8");
    let capture_id = captured_text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Captured "))
        .expect("capture output starts with its opaque ID");

    assert_eq!(capture_id.len(), 32);
    assert!(capture_id.bytes().all(|byte| (b'k'..=b'z').contains(&byte)));
    assert!(captured_text.contains("Package    capture_fixture 0.1.0"));
    assert!(captured_text.contains("Target     lib capture_fixture"));
    assert!(captured_text.contains("Profile    release"));
    assert!(
        temporary
            .path()
            .join("target/release/libcapture_fixture.rlib")
            .is_file()
    );

    let listed = run(temporary.path(), ["list-captures"]);
    assert_success(&listed);
    let listed_text = String::from_utf8(listed.stdout).expect("listing output is UTF-8");

    assert!(listed_text.starts_with("Captures\n\n"));
    assert!(listed_text.contains(&format!("Capture {capture_id}")));
    assert!(listed_text.contains("Package    capture_fixture 0.1.0"));
    assert!(listed_text.contains("Target     lib capture_fixture"));
}

#[test]
fn forwards_named_feature_and_no_default_feature_selection() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());

    let captured = run(
        temporary.path(),
        [
            "capture",
            "-p",
            "capture_fixture",
            "--bin",
            "feature-gated",
            "--release",
            "--features",
            "gated",
            "--no-default-features",
        ],
    );

    assert_success(&captured);
    assert!(
        temporary
            .path()
            .join(format!(
                "target/release/feature-gated{}",
                std::env::consts::EXE_SUFFIX
            ))
            .is_file()
    );
}

#[test]
fn forwards_all_feature_selection() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());

    let captured = run(
        temporary.path(),
        [
            "capture",
            "-p",
            "capture_fixture",
            "--bin",
            "feature-gated",
            "--release",
            "--all-features",
        ],
    );

    assert_success(&captured);
}

#[test]
fn exits_successfully_when_stdout_closes() {
    let temporary = tempfile::tempdir().expect("the test directory can be created");
    copy_fixture(temporary.path());
    let mut child = command(temporary.path())
        .arg("list-captures")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the Cargo Optic binary can run");
    drop(child.stdout.take());

    let output = child
        .wait_with_output()
        .expect("the Cargo Optic binary can exit");

    assert_success(&output);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}
