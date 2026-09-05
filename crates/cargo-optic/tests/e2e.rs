//! Protects user-visible command behavior across process boundaries.
//!
//! Each test runs the built executable against an isolated workspace.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::run as run_command;

fn run<I, S>(workspace: &TestWorkspace, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = command(workspace);
    command.args(arguments);
    let output = run_command(&mut command);
    assert_success(&command, &output);

    output
}

fn command(workspace: &TestWorkspace) -> Command {
    let mut command = Command::new("cargo");
    workspace.apply(&mut command);
    let executable = Path::new(env!("CARGO_BIN_EXE_cargo-optic"));
    let path = command
        .get_envs()
        .find(|(name, _)| *name == "PATH")
        .unwrap()
        .1
        .unwrap();
    let mut paths = vec![executable.parent().unwrap().to_owned()];
    paths.extend(std::env::split_paths(path));
    command
        .env("PATH", std::env::join_paths(paths).unwrap())
        .arg("optic");

    command
}

struct CapturedGenericFixture {
    /// Keeps the captured workspace and its store alive for each command.
    workspace: TestWorkspace,
    /// The opaque ID parsed from the successful capture output.
    capture_id: String,
    /// The original capture output retained for capture-specific assertions.
    capture_output: String,
}

impl CapturedGenericFixture {
    fn new() -> Self {
        let workspace = TestWorkspace::new("capture");

        let captured = run(
            &workspace,
            [
                "capture",
                "-p",
                "capture_fixture",
                "--bin",
                "generic",
                "--release",
            ],
        );
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
        let mut command = command(&self.workspace);
        command
            .arg("find")
            .arg("--capture")
            .arg(&self.capture_id)
            .args(arguments);
        let output = run_command(&mut command);
        assert_success(&command, &output);

        output
    }
}

fn instance_names(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("Instance "))
        .collect()
}

#[test]
fn exposes_the_cargo_subcommand_entry_point() {
    let temporary = TestWorkspace::new("capture");

    let output = run(&temporary, ["--help"]);
    let stdout = String::from_utf8(output.stdout).expect("help output is UTF-8");

    assert!(stdout.contains("Usage: cargo optic <COMMAND>"));
}

#[test]
fn reports_an_empty_capture_history() {
    let temporary = TestWorkspace::new("capture");

    let output = run(&temporary, ["list-captures"]);

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
    assert!(
        fixture
            .workspace
            .target()
            .join(format!("release/generic{}", std::env::consts::EXE_SUFFIX))
            .is_file()
    );

    let listed = run(&fixture.workspace, ["list-captures"]);
    let listed_text = String::from_utf8(listed.stdout).expect("listing output is UTF-8");

    assert!(listed_text.starts_with("Captures\n\n"));
    assert!(listed_text.contains(&format!("Capture {}", fixture.capture_id)));
    assert!(listed_text.contains("Package    capture_fixture 0.1.0"));
    assert!(listed_text.contains("Target     bin generic"));
}

#[test]
fn finds_and_renders_concrete_instances() {
    let fixture = CapturedGenericFixture::new();
    let found = fixture.find(&["kernel"]);
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

    assert_eq!(missing.stdout, b"No instances found.\n");
}

#[test]
fn captures_the_documented_library_target() {
    let temporary = TestWorkspace::new("capture");

    let captured = run(
        &temporary,
        ["capture", "-p", "capture_fixture", "--lib", "--release"],
    );
    let captured_text = String::from_utf8(captured.stdout).expect("capture output is UTF-8");
    assert!(captured_text.contains("Target     lib capture_fixture"));
    assert!(
        temporary
            .target()
            .join("release/libcapture_fixture.rlib")
            .is_file()
    );
}

#[test]
fn forwards_named_feature_and_no_default_feature_selection() {
    let temporary = TestWorkspace::new("capture");

    run(
        &temporary,
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
    assert!(
        temporary
            .target()
            .join(format!(
                "release/feature-gated{}",
                std::env::consts::EXE_SUFFIX
            ))
            .is_file()
    );
}

#[test]
fn forwards_all_feature_selection() {
    let temporary = TestWorkspace::new("capture");

    run(
        &temporary,
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
}

#[test]
fn exits_successfully_when_stdout_closes() {
    let temporary = TestWorkspace::new("capture");
    let mut command = command(&temporary);
    let mut child = command
        .arg("list-captures")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the Cargo Optic binary can run");
    drop(child.stdout.take());

    let output = child
        .wait_with_output()
        .expect("the Cargo Optic binary can exit");
    assert_success(&command, &output);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}
