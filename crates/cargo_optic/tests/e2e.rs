use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

#[test]
fn captures_a_fixture_target_and_lists_the_completed_record() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/capture");
    fs::copy(
        fixture.join("Cargo.toml"),
        temporary.path().join("Cargo.toml"),
    )
    .expect("the fixture manifest can be copied");
    fs::create_dir(temporary.path().join("src"))
        .expect("the fixture source directory can be created");
    fs::copy(
        fixture.join("src/lib.rs"),
        temporary.path().join("src/lib.rs"),
    )
    .expect("the fixture source can be copied");

    let captured = run(
        temporary.path(),
        ["capture", "-p", "capture_fixture", "--release", "lib"],
    );
    assert_success(&captured);
    let captured_text = String::from_utf8(captured.stdout).expect("capture output is UTF-8");
    let capture_id = captured_text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Captured "))
        .expect("capture output starts with its opaque ID");

    assert!(capture_id.starts_with("cap_"));
    assert!(captured_text.contains("Package    capture_fixture 0.1.0"));
    assert!(captured_text.contains("Target     lib capture_fixture"));
    assert!(captured_text.contains("Profile    release"));
    assert!(captured_text.contains("Commit     "));
    assert!(
        temporary
            .path()
            .join("target/release/libcapture_fixture.rlib")
            .is_file()
    );

    let listed = run(temporary.path(), ["captures"]);
    assert_success(&listed);
    let listed_text = String::from_utf8(listed.stdout).expect("listing output is UTF-8");

    assert!(listed_text.starts_with("Captures\n\n"));
    assert!(listed_text.contains(&format!("Capture {capture_id}")));
    assert!(listed_text.contains("Package    capture_fixture 0.1.0"));
    assert!(listed_text.contains("Target     lib capture_fixture"));
}

fn run<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_cargo-optic"))
        .arg("optic")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("the Cargo Optic binary can run")
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
