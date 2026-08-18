//! End-to-end acceptance tests for the Cargo Optic command-line workflow.
//!
//! The fixture covers concurrent capture, reuse, invalidation, lookup, and source and LLVM output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;
use walkdir::{DirEntry, WalkDir};

#[test]
fn captures_finds_and_shows_concrete_generic_instances() {
    let fixture = Fixture::new();
    let capture_arguments = [
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--format",
        "json",
    ];
    let first = fixture
        .command(capture_arguments)
        .spawn()
        .expect("the first parallel capture starts");
    let second = fixture
        .command(capture_arguments)
        .spawn()
        .expect("the second parallel capture starts");
    let first = first
        .wait_with_output()
        .expect("the first parallel capture finishes");
    let second = second
        .wait_with_output()
        .expect("the second parallel capture finishes");
    assert_success(&first);
    assert_success(&second);
    let first = json(&first);
    let second = json(&second);
    let capture = string(&first, "/result/id");
    assert_eq!(string(&second, "/result/id"), capture);
    assert_ne!(first["result"]["reused"], second["result"]["reused"]);

    fixture.append_source_comment();
    let changed = fixture.run([
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--format",
        "json",
    ]);
    assert_success(&changed);
    let changed = json(&changed);
    let changed_capture = string(&changed, "/result/id");
    assert_ne!(changed_capture, capture);
    assert_eq!(changed["result"]["reused"], false);

    let refreshed = fixture.run([
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--fresh",
        "--format",
        "json",
    ]);
    assert_success(&refreshed);
    let refreshed = json(&refreshed);
    let capture = string(&refreshed, "/result/id");
    assert_ne!(capture, changed_capture);
    assert_eq!(refreshed["result"]["reused"], false);

    let ambiguous = fixture.run([
        "show",
        "optic_mvp_kernel::outlined_sum",
        "--capture",
        capture,
        "--format",
        "json",
    ]);
    assert_eq!(ambiguous.status.code(), Some(2));
    let ambiguous = json(&ambiguous);
    assert_eq!(ambiguous["error"]["code"], "ambiguous");
    assert_eq!(
        ambiguous["error"]["result"]["instances"]
            .as_array()
            .expect("ambiguous output contains candidates")
            .len(),
        2
    );

    let found = fixture.run([
        "find",
        "--capture",
        capture,
        "optic_mvp_kernel::outlined_sum",
        "--format",
        "json",
    ]);
    assert_success(&found);
    let found = json(&found);
    let instances = found["result"]["instances"]
        .as_array()
        .expect("find returns an instance array");
    assert_eq!(instances.len(), 2);
    assert!(
        instances
            .iter()
            .all(|instance| instance["has_body"] == true)
    );
    let instance = instances
        .iter()
        .find(|instance| {
            instance["display_name"]
                .as_str()
                .is_some_and(|name| name.contains("u64"))
        })
        .expect("the fixture creates a u64 instance");
    let instance = instance["id"].as_str().expect("instances have string IDs");

    let plain = fixture.run(["show", "--capture", capture, "--instance", instance]);
    assert_success(&plain);
    let plain = String::from_utf8_lossy(&plain.stdout);
    assert!(plain.contains("===== llvm-pre-optimization:"));
    assert!(plain.contains("===== llvm-optimized:"));
    assert!(!plain.contains("===== source:"));

    let shown = fixture.run([
        "show",
        "--capture",
        capture,
        "--instance",
        instance,
        "--format",
        "json",
    ]);
    assert_success(&shown);
    let shown = json(&shown);
    assert!(shown["result"]["source"].is_null());
    let bodies = shown["result"]["bodies"]
        .as_array()
        .expect("show returns a body array");
    assert!(!bodies.is_empty());
    assert!(bodies.iter().any(|body| {
        body["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("define "))
    }));

    let with_source = fixture.run([
        "show",
        "--capture",
        capture,
        "--instance",
        instance,
        "--source",
        "--format",
        "json",
    ]);
    assert_success(&with_source);
    let with_source = json(&with_source);
    let source = string(&with_source, "/result/source/text");
    let expected_source = concat!(
        "/// Sums an array through a standalone compiler instance.\n",
        "///\n",
        "/// The acceptance test uses `#[inline(never)]` so each instance keeps a standalone ",
        "LLVM body.\n",
        "#[inline(never)]\n",
        "pub fn outlined_sum",
    );
    assert!(source.starts_with(expected_source));

    let failed = fixture.run(["capture", "-p", "missing-package", "--format", "json"]);
    assert!(!failed.status.success());
    assert_eq!(json(&failed)["error"]["code"], "operation_failed");

    let captures = fixture.run(["captures", "--format", "json"]);
    assert_success(&captures);
    assert_eq!(
        json(&captures)["result"]
            .as_array()
            .expect("capture output contains a capture array")
            .len(),
        3
    );
    assert!(fixture.staging_is_empty());
}

struct Fixture {
    _temporary: TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic");
        let root = temporary.path().join("generic");
        copy_tree(&source, &root);

        Self {
            _temporary: temporary,
            root,
        }
    }

    fn run<const N: usize>(&self, arguments: [&str; N]) -> Output {
        self.command(arguments)
            .output()
            .expect("the Cargo Optic binary starts")
    }

    fn command<const N: usize>(&self, arguments: [&str; N]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-optic"));
        command
            .arg("optic")
            .args(arguments)
            .current_dir(&self.root)
            .env("RUSTUP_TOOLCHAIN", "nightly-aarch64-apple-darwin")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        command
    }

    fn append_source_comment(&self) {
        let path = self.root.join("kernel/src/lib.rs");
        let mut source = fs::read_to_string(&path).expect("the fixture source is UTF-8");
        source.push_str("\n// Cache-key input.\n");
        fs::write(&path, source).expect("the test can change the fixture source");
    }

    fn staging_is_empty(&self) -> bool {
        fs::read_dir(self.root.join(".optic/staging"))
            .expect("the store contains a staging directory")
            .next()
            .is_none()
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    for entry in WalkDir::new(source)
        .into_iter()
        .filter_entry(committed_fixture_entry)
    {
        let entry = entry.expect("the committed fixture is readable");
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("fixture entries remain below the fixture root");
        let target = destination.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).expect("the test can create fixture directories");
        } else {
            fs::copy(entry.path(), &target).expect("the test can copy fixture files");
        }
    }
}

fn committed_fixture_entry(entry: &DirEntry) -> bool {
    entry.depth() == 0 || !matches!(entry.file_name().to_str(), Some(".optic" | "target"))
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

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("successful JSON output is valid")
}

fn string<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .expect("the JSON pointer selects a string")
}
