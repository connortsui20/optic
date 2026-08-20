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

    let reused_after_refresh = fixture.run(capture_arguments);
    assert_success(&reused_after_refresh);
    let reused_after_refresh = json(&reused_after_refresh);
    assert_eq!(string(&reused_after_refresh, "/result/id"), capture);
    assert_eq!(reused_after_refresh["result"]["reused"], true);

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
    assert!(instances.iter().all(has_optimized_definition));
    let reexported = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel::ReexportedKernel::identity",
        "--format",
        "json",
    ]);
    assert_success(&reexported);
    let reexported = json(&reexported);
    let reexported_instances = reexported["result"]["instances"]
        .as_array()
        .expect("find returns the re-exported instance");
    assert_eq!(reexported_instances.len(), 1);
    assert!(has_optimized_definition(&reexported_instances[0]));
    assert_eq!(
        reexported_instances[0]["definition"],
        "optic_mvp_kernel::ReexportedKernel::identity"
    );

    let generic_method = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel::GenericKernel",
        "--format",
        "json",
    ]);
    assert_success(&generic_method);
    let generic_method = json(&generic_method);
    let generic_methods = generic_method["result"]["instances"]
        .as_array()
        .expect("find returns the generic parent method");
    assert_eq!(generic_methods.len(), 1);
    assert_eq!(
        generic_methods[0]["definition"],
        "optic_mvp_kernel::GenericKernel::<T>::new"
    );
    let generic_method_id = generic_methods[0]["id"]
        .as_str()
        .expect("the generic parent method has an instance ID");
    let generic_source = fixture.run([
        "show",
        "--instance",
        generic_method_id,
        "--source",
        "--format",
        "json",
    ]);
    assert_success(&generic_source);
    let generic_source = json(&generic_source);
    assert!(string(&generic_source, "/result/source/text").contains("pub fn new(value: T)"));

    let nested = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "inline_add_one::chunk",
        "--format",
        "json",
    ]);
    assert_success(&nested);
    let nested = json(&nested);
    let nested_instances = nested["result"]["instances"]
        .as_array()
        .expect("find returns nested helper instances");
    assert_eq!(nested_instances.len(), 1);
    let nested_id = nested_instances[0]["id"]
        .as_str()
        .expect("the nested helper has an instance ID");
    let optimized = nested_instances[0]["availability"]
        .as_array()
        .expect("the nested helper has stage-specific availability")
        .iter()
        .find(|availability| availability["output"] == "llvm")
        .expect("optimized LLVM availability is present");
    assert_eq!(optimized["definitions"], 0);
    let nested_source = fixture.run([
        "show",
        "--instance",
        nested_id,
        "--source",
        "--format",
        "json",
    ]);
    assert_success(&nested_source);
    let nested_source = json(&nested_source);
    let nested_source = string(&nested_source, "/result/source/text");
    assert!(nested_source.contains("fn chunk"));
    assert!(nested_source.contains("value + T::from(1)"));

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
    assert!(source.contains("pub fn outlined_sum"));
    assert!(source.contains(".fold(T::default()"));

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

    let details = fixture.run(["inspect", "--capture", capture, "--format", "json"]);
    assert_success(&details);
    let details = json(&details);
    assert_eq!(details["result"]["summary"]["capture_profile"], "faithful");
    assert_eq!(details["result"]["request"]["package"], "optic-mvp-app");
    assert!(
        !details["result"]["artifacts"]
            .as_array()
            .expect("capture details include artifacts")
            .is_empty()
    );
    let encoded_details = serde_json::to_vec(&details).expect("capture details re-encode as JSON");
    assert!(!String::from_utf8_lossy(&encoded_details).contains("/.optic/"));

    let comparison = fixture.run([
        "compare", "--before", instance, "--after", instance, "--format", "json",
    ]);
    assert_success(&comparison);
    let comparison = json(&comparison);
    assert_eq!(comparison["result"]["delta"]["bytes"], 0);
    assert!(
        comparison["result"]["compatibility_differences"]
            .as_array()
            .expect("comparison includes compatibility dimensions")
            .is_empty()
    );

    let status = fixture.run(["status", "--format", "json"]);
    assert_success(&status);
    assert_eq!(json(&status)["result"]["captures"], 3);
    let verify = fixture.run(["verify", "--format", "json"]);
    assert_success(&verify);
    assert!(
        json(&verify)["result"]["verified_blobs"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );

    let removed = fixture.run(["remove", "--capture", first_capture, "--format", "json"]);
    assert_success(&removed);
    assert_eq!(json(&removed)["result"]["capture_id"], first_capture);
    let gc = fixture.run(["gc", "--format", "json"]);
    assert_success(&gc);
    let status = fixture.run(["status", "--format", "json"]);
    assert_success(&status);
    assert_eq!(json(&status)["result"]["captures"], 2);
    assert!(fixture.work_is_empty());

    let config = fixture.root.join(".optic/config.toml");
    let unknown = fixture.root.join(".optic/future-data");
    fs::write(&config, b"future configuration").expect("the test can create configuration");
    fs::write(&unknown, b"future data").expect("the test can create an unknown root entry");
    let store = fixture.root.join(".optic/store");
    let expected_store = store
        .canonicalize()
        .expect("the evidence-store path is canonical");
    let catalog = store.join("catalog.sqlite");
    let connection = Connection::open(catalog).expect("the test can open the evidence catalog");
    connection
        .pragma_update(None, "user_version", 999)
        .expect("the test can create an unsupported store version");
    drop(connection);

    let clean = fixture.run(["clean", "--format", "json"]);
    assert_success(&clean);
    assert_eq!(json(&clean)["result"]["removed"], true);
    assert_eq!(
        json(&clean)["result"]["path"],
        expected_store.to_string_lossy().as_ref()
    );
    assert!(!fixture.root.join(".optic/store").exists());
    assert!(fixture.root.join(".optic/locks/operation.lock").is_file());
    assert_eq!(
        fs::read(&config).expect("configuration remains"),
        b"future configuration"
    );
    assert_eq!(
        fs::read(&unknown).expect("the unknown root entry remains"),
        b"future data"
    );
    assert!(fixture.root.join("target").is_dir());

    let clean_again = fixture.run(["clean", "--format", "json"]);
    assert_success(&clean_again);
    assert_eq!(json(&clean_again)["result"]["removed"], false);
}

#[test]
fn cargo_observed_reuse_tracks_non_rust_and_environment_inputs() {
    let fixture = Fixture::new();
    let arguments = [
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--format",
        "json",
    ];
    let first = fixture.run(arguments);
    assert_success(&first);
    let first = json(&first);
    let first_id = string(&first, "/result/id");

    let reused = fixture.run(arguments);
    assert_success(&reused);
    let reused = json(&reused);
    assert_eq!(string(&reused, "/result/id"), first_id);
    assert_eq!(reused["result"]["reused"], true);

    fs::write(fixture.root.join("kernel/src/build-data.txt"), "second\n")
        .expect("the test can change the included compiler input");
    let included = fixture.run(arguments);
    assert_success(&included);
    let included = json(&included);
    let included_id = string(&included, "/result/id");
    assert_ne!(included_id, first_id);
    assert_eq!(included["result"]["reused"], false);

    let environment = fixture
        .command(arguments)
        .env("OPTIC_TEST_VALUE", "second")
        .output()
        .expect("the Cargo Optic binary starts");
    assert_success(&environment);
    let environment = json(&environment);
    assert_ne!(string(&environment, "/result/id"), included_id);
    assert_eq!(environment["result"]["reused"], false);
}

#[test]
fn malformed_arguments_use_the_json_error_contract() {
    for format in [
        ["--format", "json"].as_slice(),
        ["--format=json"].as_slice(),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-optic"))
            .arg("optic")
            .arg("unknown-command")
            .args(format)
            .output()
            .expect("the Cargo Optic binary starts");

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let output = json(&output);
        assert_eq!(output["version"], 2);
        assert_eq!(output["ok"], false);
        assert_eq!(output["error"]["code"], "invalid_arguments");
    }
}

#[test]
fn rustc_arguments_require_the_experiment_profile() {
    let fixture = Fixture::new();
    let output = fixture.run([
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--rustc-arg=-Ctarget-cpu=native",
        "--format",
        "json",
    ]);

    assert!(!output.status.success());
    assert!(
        string(&json(&output), "/error/message")
            .contains("--rustc-arg requires --evidence-profile experiment")
    );
}

#[cfg(unix)]
#[test]
fn preserves_compiler_wrappers_and_reuses_dependency_artifacts() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let global_wrapper = fixture.root.join("global-wrapper.sh");
    let workspace_wrapper = fixture.root.join("workspace-wrapper.sh");
    let log = fixture.root.join("wrapper.log");
    let wrapper_source = concat!(
        "#!/bin/sh\n",
        "crate=\n",
        "previous=\n",
        "for argument do\n",
        "  if [ \"$previous\" = --crate-name ]; then crate=$argument; break; fi\n",
        "  previous=$argument\n",
        "done\n",
        "kind=${0##*/}\n",
        "if [ -n \"$crate\" ]; then printf '%s:%s\\n' \"$kind\" \"$crate\" >> ",
        "\"$OPTIC_TEST_WRAPPER_LOG\"; fi\n",
        "exec \"$@\"\n",
    );
    for wrapper in [&global_wrapper, &workspace_wrapper] {
        fs::write(wrapper, wrapper_source).expect("the test can write the compiler wrapper");
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700))
            .expect("the test can make the compiler wrapper executable");
    }

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let warm = Command::new(cargo)
        .arg("build")
        .args(["-p", "optic-mvp-app", "--bin", "optic-mvp-app", "--release"])
        .current_dir(&fixture.root)
        .env("RUSTUP_TOOLCHAIN", "nightly")
        .env("RUSTC_WRAPPER", &global_wrapper)
        .env("RUSTC_WORKSPACE_WRAPPER", &workspace_wrapper)
        .env("OPTIC_TEST_WRAPPER_LOG", &log)
        .env("OPTIC_TEST_VALUE", "first")
        .output()
        .expect("the warm Cargo build starts");
    assert_success(&warm);
    fs::write(&log, []).expect("the test can clear the compiler wrapper log");

    let capture = fixture
        .command([
            "capture",
            "-p",
            "optic-mvp-app",
            "--bin",
            "optic-mvp-app",
            "--release",
            "--fresh",
            "--format",
            "json",
        ])
        .env("RUSTC_WRAPPER", &global_wrapper)
        .env("RUSTC_WORKSPACE_WRAPPER", &workspace_wrapper)
        .env("OPTIC_TEST_WRAPPER_LOG", &log)
        .output()
        .expect("the Cargo Optic capture starts");
    assert_success(&capture);
    let wrapper_log = fs::read_to_string(&log).expect("the compiler wrapper log is readable");
    let compiler_invocations = wrapper_log
        .lines()
        .filter(|invocation| !invocation.ends_with(":___"))
        .collect::<Vec<_>>();

    assert_eq!(
        compiler_invocations,
        [
            "global-wrapper.sh:optic_mvp_app",
            "workspace-wrapper.sh:optic_mvp_app"
        ]
    );
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
            .env("OPTIC_TEST_VALUE", "first")
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

    fn work_is_empty(&self) -> bool {
        fs::read_dir(self.root.join(".optic/store/work"))
            .expect("the store contains a work directory")
            .next()
            .is_none()
    }
}

#[track_caller]
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

fn has_optimized_definition(instance: &Value) -> bool {
    instance["availability"]
        .as_array()
        .expect("instances include stage-specific availability")
        .iter()
        .any(|availability| {
            availability["output"] == "llvm"
                && availability["definitions"]
                    .as_u64()
                    .is_some_and(|count| count > 0)
        })
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
