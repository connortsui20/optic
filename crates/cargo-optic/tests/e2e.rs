//! Protects user-visible command behavior across process boundaries.
//!
//! Each test runs the built executable against an isolated workspace.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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

fn capture_show(workspace: &TestWorkspace, profile: &str) -> String {
    let output = run(
        workspace,
        [
            "capture",
            "-p",
            "show_fixture",
            "--bin",
            "show_fixture",
            "--profile",
            profile,
        ],
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    stdout
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Captured ")
        .unwrap()
        .to_owned()
}

#[track_caller]
fn find_reference(workspace: &TestWorkspace, capture: &str, query: &str) -> String {
    let output = run(workspace, ["find", "--capture", capture, query]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let references = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("  Reference   "))
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 1, "{stdout}");

    references[0].to_owned()
}

#[test]
fn direct_cli_skips_non_executable_cargo_on_path() {
    let workspace = TestWorkspace::new("capture");
    let shadow = workspace.observations().join("shadow");
    fs::create_dir(&shadow).unwrap();
    let cargo = shadow.join("cargo");
    fs::write(&cargo, "not an executable\n").unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o644)).unwrap();

    let mut ordinary = Command::new("cargo");
    workspace.apply(&mut ordinary);
    let path = ordinary
        .get_envs()
        .find(|(name, _)| *name == "PATH")
        .unwrap()
        .1
        .unwrap();
    let paths = std::iter::once(shadow).chain(std::env::split_paths(path));
    let path = std::env::join_paths(paths).unwrap();
    ordinary.env("PATH", &path).env_remove("CARGO").arg("-V");
    let output = run_command(&mut ordinary);
    assert_success(&ordinary, &output);

    for cargo in [None, Some(""), Some("cargo")] {
        let mut direct = Command::new(env!("CARGO_BIN_EXE_cargo-optic"));
        workspace.apply(&mut direct);
        direct.env("PATH", &path).args(["optic", "list-captures"]);
        match cargo {
            Some(cargo) => direct.env("CARGO", cargo),
            None => direct.env_remove("CARGO"),
        };
        let output = run_command(&mut direct);
        assert_success(&direct, &output);
    }
}

#[test]
fn direct_cli_keeps_path_order_and_the_cargo_symlink_name() {
    let workspace = TestWorkspace::new("capture");
    let shim = workspace.observations().join("dispatch");
    fs::write(
        &shim,
        "#!/bin/sh\nprintf 'selected Cargo: %s\\n' \"$0\" >&2\nexit 41\n",
    )
    .unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let first = workspace.workspace().join("tools");
    fs::create_dir(&first).unwrap();
    let cargo = first.join("cargo");
    std::os::unix::fs::symlink(&shim, &cargo).unwrap();

    for entry in [first.as_path(), Path::new("tools")] {
        let mut direct = Command::new(env!("CARGO_BIN_EXE_cargo-optic"));
        workspace.apply(&mut direct);
        let path = direct
            .get_envs()
            .find(|(name, _)| *name == "PATH")
            .unwrap()
            .1
            .unwrap();
        let paths = std::iter::once(entry.to_owned()).chain(std::env::split_paths(path));
        let path = std::env::join_paths(paths).unwrap();
        direct
            .env("PATH", path)
            .env_remove("CARGO")
            .args(["optic", "list-captures"]);
        let output = run_command(&mut direct);

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains(&format!("selected Cargo: {}", cargo.display()))
        );
    }
}

#[test]
fn direct_cli_does_not_replace_explicit_non_executable_cargo() {
    let workspace = TestWorkspace::new("capture");
    let cargo = workspace.workspace().join("chosen-cargo");
    fs::write(&cargo, "not an executable\n").unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o644)).unwrap();

    for selected in [cargo.as_os_str(), OsStr::new("./chosen-cargo")] {
        let mut direct = Command::new(env!("CARGO_BIN_EXE_cargo-optic"));
        workspace.apply(&mut direct);
        direct
            .env("CARGO", selected)
            .args(["optic", "list-captures"]);
        let output = run_command(&mut direct);

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Permission denied"));
    }
}

#[test]
fn shows_exact_api_bytes_after_checkout_changes() {
    let workspace = TestWorkspace::new("show");
    let capture = capture_show(&workspace, "release");
    let reference = find_reference(&workspace, &capture, "show_fixture::source_items::ordinary");
    let mut expected = Command::new(std::env::current_exe().unwrap());
    workspace.apply(&mut expected);
    expected
        .args(["--exact", "write_api_evidence_in_child", "--nocapture"])
        .env("OPTIC_TEST_INSTANCE", &reference)
        .env("OPTIC_TEST_EVIDENCE", workspace.observations());
    let output = run_command(&mut expected);
    assert_success(&expected, &output);
    let source_bytes = fs::read(workspace.observations().join("source")).unwrap();
    let llvm_bytes = fs::read(workspace.observations().join("llvm")).unwrap();
    assert!(!llvm_bytes.is_empty());
    let whole_function =
        fs::read_to_string(workspace.workspace().join("expected/ordinary.txt")).unwrap();
    assert_eq!(
        source_bytes,
        whole_function.strip_suffix('\n').unwrap().as_bytes()
    );
    fs::write(
        workspace.workspace().join("src/source_items.rs"),
        "pub fn broken( {\n",
    )
    .unwrap();
    let drivers = workspace.cargo_home().join("optic/drivers");
    assert!(drivers.is_dir());
    fs::rename(&drivers, workspace.observations().join("retained-drivers")).unwrap();

    for (format, bytes, diagnostic) in [
        ("source", source_bytes, "Source "), // Exact captured function bytes.
        ("llvm", llvm_bytes, "LLVM "),       // Exact API body bytes and separators.
    ] {
        let output = run(
            &workspace,
            ["show", "--instance", &reference, "--output", format],
        );
        assert_eq!(output.stdout, bytes, "{format}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with(diagnostic)
        );
    }

    assert!(!drivers.exists());
    let output = run(&workspace, ["list-captures"]);
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("Capture "))
            .count(),
        1
    );
}

#[test]
fn show_exits_successfully_when_stdout_closes() {
    let workspace = TestWorkspace::new("show");
    let capture = capture_show(&workspace, "release");
    let reference = find_reference(&workspace, &capture, "show_fixture::source_items::ordinary");

    for format in ["source", "llvm"] {
        let mut command = command(&workspace);
        command
            .args(["show", "--instance", &reference, "--output", format])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();

        assert_success(&command, &output);
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("panicked")
        );
    }
}

#[test]
fn unavailable_show_has_empty_stdout_and_a_failing_status() {
    let workspace = TestWorkspace::new("show");
    let capture = capture_show(&workspace, "incremental");

    for (query, format, reason) in [
        ("show_dependency::external_generic", "source", "nonlocal"), // Unsupported source provenance.
        (
            "show_fixture::source_items::generated",
            "source",
            "expansion",
        ), // Generated function source.
        (
            "show_fixture::source_items::ordinary",
            "llvm",
            "incremental",
        ), // Unsupported LLVM configuration.
    ] {
        let reference = find_reference(&workspace, &capture, query);
        let mut command = command(&workspace);
        command.args(["show", "--instance", &reference, "--output", format]);
        let output = run_command(&mut command);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(&reference), "{stderr}");
        assert!(stderr.contains(reason), "{stderr}");
    }
}

#[test]
fn write_api_evidence_in_child() {
    let Ok(reference) = std::env::var("OPTIC_TEST_INSTANCE") else {
        return;
    };
    let reference = reference.parse::<optic::InstanceRef>().unwrap();
    let optic = optic::Optic::open(&std::env::current_dir().unwrap()).unwrap();
    let directory = std::path::PathBuf::from(std::env::var_os("OPTIC_TEST_EVIDENCE").unwrap());
    let optic::SourceEvidence::Available { evidence, .. } = optic.source(&reference).unwrap()
    else {
        panic!("expected stored function source");
    };
    let mut source = Vec::new();
    optic.copy_evidence(&evidence, &mut source).unwrap();
    fs::write(directory.join("source"), source).unwrap();
    let optic::LlvmEvidence::Available(bodies) = optic.llvm(&reference).unwrap() else {
        panic!("expected stored optimized LLVM");
    };
    assert!(!bodies.is_empty());
    let mut llvm = Vec::new();

    for (index, body) in bodies.iter().enumerate() {
        if index != 0 {
            llvm.push(b'\n');
        }

        optic.copy_evidence(body.evidence(), &mut llvm).unwrap();
    }

    fs::write(directory.join("llvm"), llvm).unwrap();
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
fn reports_reuse_and_forced_capture_through_cargo_discovery() {
    let fixture = CapturedGenericFixture::new();
    let arguments = [
        "capture",
        "-p",
        "capture_fixture",
        "--bin",
        "generic",
        "--release",
    ];
    let reused = run(&fixture.workspace, arguments);
    let reused_text = String::from_utf8(reused.stdout).unwrap();
    assert_eq!(
        reused_text.lines().next().unwrap(),
        format!("Reused {}", fixture.capture_id)
    );

    let listed = run(&fixture.workspace, ["list-captures"]);
    let listed_text = String::from_utf8(listed.stdout).unwrap();
    assert_eq!(
        listed_text
            .lines()
            .filter(|line| line.starts_with("Capture "))
            .count(),
        1
    );

    let fresh = run(&fixture.workspace, arguments.into_iter().chain(["--fresh"]));
    let fresh_text = String::from_utf8(fresh.stdout).unwrap();
    let fresh_id = fresh_text
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Captured ")
        .unwrap();
    assert_ne!(fresh_id, fixture.capture_id);
    let repeated = run(&fixture.workspace, arguments);
    let repeated_text = String::from_utf8(repeated.stdout).unwrap();
    assert_eq!(
        repeated_text.lines().next().unwrap(),
        format!("Reused {fresh_id}")
    );

    let listed = run(&fixture.workspace, ["list-captures"]);
    let listed_text = String::from_utf8(listed.stdout).unwrap();
    assert_eq!(
        listed_text
            .lines()
            .filter(|line| line.starts_with("Capture "))
            .count(),
        2
    );
    assert!(listed_text.contains(&format!("Capture {}", fixture.capture_id)));
    assert!(listed_text.contains(&format!("Capture {fresh_id}")));
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
