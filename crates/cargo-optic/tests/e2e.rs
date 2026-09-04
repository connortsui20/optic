//! Protects user-visible command behavior across process boundaries.
//!
//! Each test runs the built executable against an isolated workspace.

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

    // These tests assert fixture artifacts below `target/release`. Remove the developer's target
    // override so it cannot redirect those artifacts outside the isolated workspace.
    command
        .current_dir(directory)
        .env_remove("CARGO_TARGET_DIR")
        .arg("optic");

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
        fixture.join("src/generic.rs"),
        directory.join("src/generic.rs"),
    )
    .expect("the generic fixture source can be copied");
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

struct CapturedGenericFixture {
    /// Keeps the captured workspace and its store alive for each command.
    workspace: tempfile::TempDir,
    /// The opaque ID parsed from the successful capture output.
    capture_id: String,
    /// The original capture output retained for capture-specific assertions.
    capture_output: String,
}

impl CapturedGenericFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("the test workspace can be created");
        copy_fixture(workspace.path());

        let captured = run(
            workspace.path(),
            [
                "capture",
                "-p",
                "capture_fixture",
                "--bin",
                "generic",
                "--release",
            ],
        );
        assert_success(&captured);
        let capture_output = String::from_utf8(captured.stdout).expect("capture output is UTF-8");
        let capture_id = capture_output
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("Captured "))
            .expect("capture output starts with its opaque ID")
            .to_owned();

        Self {
            workspace,
            capture_id,
            capture_output,
        }
    }

    fn find(&self, arguments: &[&str]) -> Output {
        command(self.workspace.path())
            .arg("find")
            .arg("--capture")
            .arg(&self.capture_id)
            .args(arguments)
            .output()
            .expect("the Cargo Optic find command can run")
    }
}

fn instance_names(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("Instance "))
        .collect()
}

fn capture_count(output: &str, field: &str) -> u64 {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix(field))
        .map(str::trim)
        .expect("capture output contains the requested count")
        .parse()
        .expect("the capture count is an unsigned integer")
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
fn captures_and_lists_a_generic_fixture_target() {
    let fixture = CapturedGenericFixture::new();

    assert_eq!(fixture.capture_id.len(), 32);
    assert!(
        fixture
            .capture_id
            .bytes()
            .all(|byte| (b'k'..=b'z').contains(&byte))
    );
    assert!(
        fixture
            .capture_output
            .contains("Package    capture_fixture 0.1.0")
    );
    assert!(fixture.capture_output.contains("Target     bin generic"));
    assert!(fixture.capture_output.contains("Profile    release"));
    let instance_count = capture_count(&fixture.capture_output, "Instances");
    let placement_count = capture_count(&fixture.capture_output, "Placements");
    assert!(instance_count > 0);
    assert!(placement_count >= instance_count);
    assert!(
        fixture
            .workspace
            .path()
            .join(format!(
                "target/release/generic{}",
                std::env::consts::EXE_SUFFIX
            ))
            .is_file()
    );

    let listed = run(fixture.workspace.path(), ["list-captures"]);
    assert_success(&listed);
    let listed_text = String::from_utf8(listed.stdout).expect("listing output is UTF-8");

    assert!(listed_text.starts_with("Captures\n\n"));
    assert!(listed_text.contains(&format!("Capture {}", fixture.capture_id)));
    assert!(listed_text.contains("Package    capture_fixture 0.1.0"));
    assert!(listed_text.contains("Target     bin generic"));
    assert_eq!(capture_count(&listed_text, "Instances"), instance_count);
    assert_eq!(capture_count(&listed_text, "Placements"), placement_count);
}

#[test]
fn finds_and_renders_concrete_instances() {
    let fixture = CapturedGenericFixture::new();
    let found = fixture.find(&["kernel"]);
    assert_success(&found);
    let found_text = String::from_utf8(found.stdout).expect("find output is UTF-8");
    let names = instance_names(&found_text);
    let outlined_instances = names
        .iter()
        .copied()
        .filter(|name| name.contains("outlined_kernel"))
        .collect::<Vec<_>>();

    assert!(
        outlined_instances.len() >= 2,
        "expected two outlined instances, got:\n{found_text}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.contains("nested_kernel::chunk")),
        "expected the nested generic instance, got:\n{found_text}"
    );
    assert!(found_text.contains(&format!("  Capture     {}", fixture.capture_id)));
    assert!(found_text.contains("  Definition  generic::outlined_kernel"));
    assert!(found_text.contains("  Symbol      "));
    assert!(found_text.contains("  Placement   "));
}

#[test]
fn applies_the_find_result_limit() {
    let fixture = CapturedGenericFixture::new();
    let limited = fixture.find(&["--limit", "1", "kernel"]);

    assert_success(&limited);
    let limited_text = String::from_utf8(limited.stdout).expect("limited output is UTF-8");
    let total_matches = limited_text
        .lines()
        .last()
        .and_then(|line| line.strip_prefix("Showing 1 of "))
        .and_then(|line| {
            line.strip_suffix(" matching instances. Narrow the query to reduce the result set.")
        })
        .expect("limited output ends with its truncation notice")
        .parse::<usize>()
        .expect("the total match count is an unsigned integer");

    assert_eq!(limited_text.matches("Instance ").count(), 1);
    assert!(limited_text.contains(&format!("  Capture     {}", fixture.capture_id)));
    assert!(total_matches > 1);
}

#[test]
fn reports_when_no_instances_match() {
    let fixture = CapturedGenericFixture::new();
    let missing = fixture.find(&["not_a_compiler_instance"]);

    assert_success(&missing);

    assert_eq!(missing.stdout, b"No instances found.\n");
}

#[test]
fn rejects_an_excessive_find_result_limit() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());
    let excessive_limit = run(
        temporary.path(),
        [
            "find",
            "--capture",
            "zyxwvutsrqponmlkzyxwvutsrqponmlk",
            "--limit",
            "1001",
            "kernel",
        ],
    );

    assert!(!excessive_limit.status.success());
    assert!(
        String::from_utf8_lossy(&excessive_limit.stderr)
            .contains("instance result limit must be between 1 and 1000, got 1001")
    );
}

#[test]
fn captures_the_documented_library_target() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());

    let captured = run(
        temporary.path(),
        ["capture", "-p", "capture_fixture", "--lib", "--release"],
    );

    assert_success(&captured);
    let captured_text = String::from_utf8(captured.stdout).expect("capture output is UTF-8");
    assert!(captured_text.contains("Target     lib capture_fixture"));
    assert!(
        temporary
            .path()
            .join("target/release/libcapture_fixture.rlib")
            .is_file()
    );
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
