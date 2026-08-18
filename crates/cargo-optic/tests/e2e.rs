//! End-to-end acceptance tests for the Cargo Optic command-line workflow.
//!
//! The fixture covers concurrent capture, reuse, invalidation, lookup, and source and LLVM output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use rusqlite::Connection;
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
    let first_capture = string(&first, "/result/id");
    assert_eq!(string(&second, "/result/id"), first_capture);
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
    assert_ne!(changed_capture, first_capture);
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
    let capture_ids = [first_capture, changed_capture, capture];
    let capture_prefix = shortest_unique_prefix(capture, &capture_ids);
    let capture_display = displayed_id(capture, &capture_ids);

    let ambiguous = fixture.run([
        "show",
        "optic_mvp_kernel::outlined_sum",
        "--capture",
        capture_prefix,
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

    let ambiguous_text = fixture.run([
        "show",
        "optic_mvp_kernel::outlined_sum",
        "--capture",
        capture_prefix,
        "--output",
        "llvm-pre-opt",
        "--source",
    ]);
    assert_eq!(ambiguous_text.status.code(), Some(2));
    let ambiguous_text = String::from_utf8_lossy(&ambiguous_text.stdout);
    assert!(ambiguous_text.contains("Run one command"));
    assert!(ambiguous_text.contains("--output llvm-pre-opt --source"));

    let found = fixture.run([
        "find",
        "--capture",
        capture_prefix,
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
    let instance_ids = [first_capture, changed_capture, capture]
        .into_iter()
        .flat_map(|capture_id| stored_instance_ids(&fixture, capture_id))
        .collect::<Vec<_>>();
    let instance_id_refs = instance_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let instance = instances
        .iter()
        .find(|instance| {
            instance["display_name"]
                .as_str()
                .is_some_and(|name| name.contains("u64"))
        })
        .expect("the fixture creates a u64 instance");
    let instance = instance["id"].as_str().expect("instances have string IDs");
    let instance_prefix = shortest_unique_prefix(instance, &instance_id_refs);
    let instance_display = displayed_id(instance, &instance_id_refs);

    let found_text = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel::outlined_sum",
    ]);
    assert_success(&found_text);
    let found_text = String::from_utf8_lossy(&found_text.stdout);
    assert!(found_text.contains(&format!("cargo optic show --instance {instance_display}")));
    assert!(ambiguous_text.contains(&format!(
        "cargo optic show --instance {instance_display} --output llvm-pre-opt --source"
    )));

    let plain = fixture.run(["show", "--instance", instance_prefix]);
    assert_success(&plain);
    let plain = String::from_utf8_lossy(&plain.stdout);
    assert!(plain.contains(&format!("  Capture   {capture_display}")));
    assert!(plain.contains(&format!("  Instance  {instance_display}")));
    assert!(plain.contains("LLVM (optimized)  "));
    assert!(!plain.contains("llvm-pre-opt"));
    assert!(!plain.contains("Source  "));
    assert!(!plain.contains('\x1b'));

    let colored = fixture.run([
        "show",
        "--instance",
        instance_prefix,
        "--source",
        "--color",
        "always",
    ]);
    assert_success(&colored);
    let colored = String::from_utf8_lossy(&colored.stdout);
    assert!(colored.contains("\x1b["));
    assert!(colored.contains("\x1b[38;2;"));
    assert!(colored.contains("Source  "));
    assert!(colored.contains(&format!("\x1b[1m\x1b[93m{instance_prefix}\x1b[0m")));
    assert!(colored.contains(&format!(
        "\x1b[90m{}\x1b[0m",
        &instance_display[instance_prefix.len()..]
    )));

    let shown = fixture.run([
        "show",
        "--instance",
        instance_prefix,
        "--format",
        "json",
        "--color",
        "always",
    ]);
    assert_success(&shown);
    assert!(!String::from_utf8_lossy(&shown.stdout).contains("\\u001b"));
    let shown = json(&shown);
    assert_eq!(shown["result"]["capture_id"], capture);
    assert_eq!(shown["result"]["instance"]["id"], instance);
    assert_eq!(shown["result"]["output"], "llvm");
    assert!(shown["result"]["source"].is_null());
    let bodies = shown["result"]["bodies"]
        .as_array()
        .expect("show returns a body array");
    assert!(!bodies.is_empty());
    assert!(bodies.iter().all(|body| body["stage"] == "llvm-optimized"));
    assert!(bodies.iter().all(|body| {
        body["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("define "))
    }));
    let pre_optimization = fixture.run([
        "show",
        "--instance",
        instance_prefix,
        "--output",
        "llvm-pre-opt",
        "--format",
        "json",
    ]);
    assert_success(&pre_optimization);
    let pre_optimization = json(&pre_optimization);
    assert_eq!(pre_optimization["result"]["output"], "llvm-pre-opt");
    assert!(
        pre_optimization["result"]["bodies"]
            .as_array()
            .expect("show returns a body array")
            .iter()
            .all(|body| body["stage"] == "llvm-pre-optimization")
    );

    let with_source = fixture.run([
        "show",
        "--instance",
        instance_prefix,
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

    let redundant_capture = fixture.run([
        "show",
        "--capture",
        capture_prefix,
        "--instance",
        instance_prefix,
        "--format",
        "json",
    ]);
    assert!(!redundant_capture.status.success());
    assert!(
        string(&json(&redundant_capture), "/error/message")
            .contains("--instance cannot be combined with --capture")
    );

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

    let catalog = fixture.root.join(".optic/catalog.sqlite");
    let connection = Connection::open(catalog).expect("the test can open the evidence catalog");
    connection
        .pragma_update(None, "user_version", 999)
        .expect("the test can create an unsupported store version");
    drop(connection);

    let clean = fixture.run(["clean", "--format", "json"]);
    assert_success(&clean);
    assert_eq!(json(&clean)["result"]["removed"], true);
    assert!(!fixture.root.join(".optic").exists());
    assert!(fixture.root.join("target").is_dir());

    let clean_again = fixture.run(["clean", "--format", "json"]);
    assert_success(&clean_again);
    assert_eq!(json(&clean_again)["result"]["removed"], false);
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
            .env("RUSTUP_TOOLCHAIN", "nightly")
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
    entry.depth() == 0
        || !matches!(
            entry.file_name().to_str(),
            Some(".optic" | ".optic.lock" | "target")
        )
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

#[track_caller]
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("command JSON output is valid")
}

#[track_caller]
fn string<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .expect("the JSON pointer selects a string")
}

#[track_caller]
fn stored_instance_ids(fixture: &Fixture, capture_id: &str) -> Vec<String> {
    let output = fixture.run(["find", "--capture", capture_id, "", "--format", "json"]);
    assert_success(&output);
    let output = json(&output);

    output["result"]["instances"]
        .as_array()
        .expect("an empty query returns all instances")
        .iter()
        .map(|instance| {
            instance["id"]
                .as_str()
                .expect("instances have string IDs")
                .to_owned()
        })
        .collect()
}

#[track_caller]
fn shortest_unique_prefix<'a>(identifier: &'a str, candidates: &[&str]) -> &'a str {
    for length in 5..identifier.len() {
        let prefix = &identifier[..length];
        let match_count = candidates
            .iter()
            .filter(|candidate| candidate.starts_with(prefix))
            .count();

        if match_count == 1 {
            return prefix;
        }
    }

    identifier
}

#[track_caller]
fn displayed_id<'a>(identifier: &'a str, candidates: &[&str]) -> &'a str {
    let unique_prefix = shortest_unique_prefix(identifier, candidates);
    let type_prefix_length = identifier
        .find('_')
        .map(|index| index + 1)
        .expect("fixture IDs contain a type prefix separator");
    let minimum_length = (type_prefix_length + 12).min(identifier.len());
    let display_length = minimum_length.max(unique_prefix.len());

    &identifier[..display_length]
}
