//! End-to-end acceptance tests for the Cargo Optic command-line workflow.
//!
//! The fixture covers capture, reuse, invalidation, compiler supervision, feature selection,
//! lookup, and source and LLVM output.

use std::ffi::OsStr;
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
        "jsonl",
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
    assert_ne!(
        first["result"]["disposition"],
        second["result"]["disposition"]
    );

    fixture.append_source_comment();
    let changed = fixture.run([
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--format",
        "jsonl",
    ]);
    assert_success(&changed);
    let changed = json(&changed);
    let changed_capture = string(&changed, "/result/id");
    assert_ne!(changed_capture, first_capture);
    assert_eq!(changed["result"]["disposition"], "captured");

    let refreshed = fixture.run([
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--fresh",
        "--format",
        "jsonl",
    ]);
    assert_success(&refreshed);
    let refreshed = json(&refreshed);
    let capture = string(&refreshed, "/result/id");
    assert_ne!(capture, changed_capture);
    assert_eq!(refreshed["result"]["disposition"], "captured");

    let reused_after_refresh = fixture.run(capture_arguments);
    assert_success(&reused_after_refresh);
    let reused_after_refresh = json(&reused_after_refresh);
    assert_eq!(string(&reused_after_refresh, "/result/id"), capture);
    assert_eq!(reused_after_refresh["result"]["disposition"], "reused");

    let reused_over_budget = fixture.run([
        "show",
        "optic_mvp_kernel::ReexportedKernel::identity",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--max-store-bytes",
        "0",
        "--format",
        "jsonl",
    ]);
    assert_success(&reused_over_budget);
    assert_eq!(
        string(&json(&reused_over_budget), "/result/capture_id"),
        capture
    );

    let capture_ids = [first_capture, changed_capture, capture];
    let capture_prefix = shortest_unique_prefix(capture, &capture_ids);
    let capture_display = displayed_id(capture, &capture_ids);

    let ambiguous = fixture.run([
        "show",
        "optic_mvp_kernel::outlined_sum",
        "--capture",
        capture_prefix,
        "--format",
        "jsonl",
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
        "jsonl",
    ]);
    assert_success(&found);
    let found = json(&found);
    assert_eq!(found["result"]["match_kind"], "exact");
    assert_eq!(found["result"]["truncated"], false);
    let instances = found["result"]["instances"]
        .as_array()
        .expect("find returns an instance array");
    assert_eq!(instances.len(), 2);
    assert!(instances.iter().all(has_optimized_definition));
    assert!(instances.iter().all(|instance| {
        instance["compiler_symbol"]
            .as_str()
            .is_some_and(|symbol| !symbol.is_empty())
            && instance["symbol_fingerprint"]
                .as_str()
                .is_some_and(|fingerprint| {
                    fingerprint.len() == 12
                        && fingerprint
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
    }));
    let reexported = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel::ReexportedKernel::identity",
        "--format",
        "jsonl",
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
    let filtered = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel::ReexportedKernel::identity",
        "--crate",
        "optic_mvp_kernel",
        "--definition",
        "optic_mvp_kernel::ReexportedKernel::identity",
        "--available",
        "llvm",
        "--limit",
        "1",
        "--format",
        "jsonl",
    ]);
    assert_success(&filtered);
    let filtered = json(&filtered);
    assert_eq!(
        filtered["result"]["instances"]
            .as_array()
            .expect("filtered find returns candidates")
            .len(),
        1
    );

    let limited = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel",
        "--limit",
        "1",
        "--format",
        "jsonl",
    ]);
    assert_success(&limited);
    let limited = json(&limited);
    assert_eq!(limited["result"]["match_kind"], "substring");
    assert_eq!(limited["result"]["truncated"], true);
    assert_eq!(
        limited["result"]["instances"]
            .as_array()
            .expect("limited find returns candidates")
            .len(),
        1
    );

    let short_substring = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "zz",
        "--format",
        "jsonl",
    ]);
    assert!(!short_substring.status.success());
    assert!(
        String::from_utf8_lossy(&short_substring.stdout)
            .contains("substring queries must contain at least 3 Unicode characters")
    );

    let generic_method = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "optic_mvp_kernel::GenericKernel",
        "--format",
        "jsonl",
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
        "jsonl",
    ]);
    assert_success(&generic_source);
    assert_jsonl_stream(&generic_source);
    let generic_source = json(&generic_source);
    assert!(string(&generic_source, "/result/source/text").contains("pub fn new(value: T)"));

    let nested = fixture.run([
        "find",
        "--capture",
        capture_prefix,
        "inline_add_one::chunk",
        "--format",
        "jsonl",
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
        "jsonl",
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
    let optimized_availability = instance["availability"]
        .as_array()
        .expect("the instance reports output availability")
        .iter()
        .find(|availability| availability["output"] == "llvm")
        .expect("optimized LLVM availability is present");
    assert_eq!(optimized_availability["definitions"], 1);
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
        "jsonl",
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
    assert_eq!(bodies.len(), 1);
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
        "jsonl",
    ]);
    assert_success(&pre_optimization);
    let pre_optimization = json(&pre_optimization);
    assert_eq!(pre_optimization["result"]["output"], "llvm-pre-opt");
    let pre_optimization_bodies = pre_optimization["result"]["bodies"]
        .as_array()
        .expect("show returns a body array");
    assert_eq!(pre_optimization_bodies.len(), 1);
    assert!(
        pre_optimization_bodies
            .iter()
            .all(|body| body["stage"] == "llvm-pre-optimization")
    );

    let with_source = fixture.run([
        "show",
        "--instance",
        instance_prefix,
        "--source",
        "--format",
        "jsonl",
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
        "jsonl",
    ]);
    assert!(!redundant_capture.status.success());
    assert!(
        string(&json(&redundant_capture), "/error/message")
            .contains("--instance cannot be combined with --capture")
    );

    let failed = fixture.run(["capture", "-p", "missing-package", "--format", "jsonl"]);
    assert!(!failed.status.success());
    assert_eq!(json(&failed)["error"]["code"], "operation_failed");

    let captures = fixture.run(["captures", "--format", "jsonl"]);
    assert_success(&captures);
    assert_eq!(
        json(&captures)["result"]
            .as_array()
            .expect("capture output contains a capture array")
            .len(),
        3
    );

    let details = fixture.run(["inspect", "--capture", capture, "--format", "jsonl"]);
    assert_success(&details);
    let details = json(&details);
    let compiler = cargo_ir::inspect_workspace_toolchain(&fixture.root)
        .expect("the fixture compiler identity is available");
    assert_eq!(details["result"]["summary"]["capture_profile"], "faithful");
    assert_eq!(details["result"]["request"]["package"], "optic-mvp-app");
    assert_eq!(
        string(&details, "/result/compiler/rustc"),
        compiler.rustc.to_string_lossy()
    );
    assert_eq!(
        string(&details, "/result/compiler/release"),
        compiler.release
    );
    assert_eq!(
        string(&details, "/result/compiler/commit_hash"),
        compiler.commit_hash
    );
    assert_eq!(string(&details, "/result/compiler/host"), compiler.host);
    assert_eq!(
        string(&details, "/result/compiler/llvm_version"),
        compiler.llvm_version
    );
    assert_eq!(
        string(&details, "/result/compiler/sysroot"),
        compiler.sysroot.to_string_lossy()
    );
    assert_eq!(
        string(&details, "/result/compiler/llvm_dis"),
        compiler.llvm_dis.to_string_lossy()
    );
    assert_eq!(
        details["result"]["unstable_access"],
        serde_json::json!({
            "mechanism": "rustc-bootstrap",
            "authorized_scopes": [
                "cargo-config-discovery",
                "driver-build",
                "selected-target",
            ],
        })
    );
    assert!(
        !details["result"]["artifacts"]
            .as_array()
            .expect("capture details include artifacts")
            .is_empty()
    );
    let encoded_details = serde_json::to_vec(&details).expect("capture details re-encode as JSON");
    assert!(!String::from_utf8_lossy(&encoded_details).contains("/.optic/"));

    let details_text = fixture.run(["inspect", "--capture", capture]);
    assert_success(&details_text);
    let details_text = String::from_utf8_lossy(&details_text.stdout);
    assert!(details_text.contains(&format!("  Commit    {}", compiler.commit_hash)));
    assert!(details_text.contains(
        "  Unstable  rustc-bootstrap (authorized: cargo-config-discovery, driver-build, \
         selected-target)"
    ));

    let comparison = fixture.run([
        "compare", "--before", instance, "--after", instance, "--format", "jsonl",
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

    let status = fixture.run(["status", "--format", "jsonl"]);
    assert_success(&status);
    assert_eq!(json(&status)["result"]["captures"], 3);
    let verify = fixture.run(["verify", "--format", "jsonl"]);
    assert_success(&verify);
    assert!(
        json(&verify)["result"]["verified_blobs"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );

    let removed = fixture.run(["remove", "--capture", first_capture, "--format", "jsonl"]);
    assert_success(&removed);
    assert_eq!(json(&removed)["result"]["capture_id"], first_capture);
    let gc = fixture.run(["gc", "--format", "jsonl"]);
    assert_success(&gc);
    let status = fixture.run(["status", "--format", "jsonl"]);
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

    let clean = fixture.run(["clean", "--format", "jsonl"]);
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

    let clean_again = fixture.run(["clean", "--format", "jsonl"]);
    assert_success(&clean_again);
    assert_eq!(json(&clean_again)["result"]["removed"], false);
}

#[test]
fn reads_and_compares_explicit_foreign_stores() {
    let fixture = Fixture::new();
    let captured = fixture.run([
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--format",
        "jsonl",
    ]);
    assert_success(&captured);
    let capture = json(&captured);
    let capture_id = string(&capture, "/result/id");
    let selected = fixture.run([
        "find",
        "--capture",
        capture_id,
        "optic_mvp_kernel::outlined_sum",
        "--format",
        "jsonl",
    ]);
    assert_success(&selected);
    let selected = json(&selected);
    let instance_id = string(&selected, "/result/instances/0/id").to_owned();
    let before = fixture.temporary.path().join("foreign optic's/.optic");
    let after = fixture.temporary.path().join("second foreign/.optic");
    copy_directory(&fixture.root.join(".optic"), &before);
    copy_directory(&fixture.root.join(".optic"), &after);
    let before_entries = durable_paths_below(&before);
    let outside_workspace = fixture.temporary.path().join("outside-workspace");
    fs::create_dir(&outside_workspace).expect("the test can create a non-Cargo directory");
    let before = before.to_str().expect("temporary paths are UTF-8");
    let after = after.to_str().expect("temporary paths are UTF-8");

    let found = fixture
        .command([
            "--optic-dir",
            before,
            "find",
            "--capture",
            capture_id,
            "optic_mvp_kernel::outlined_sum",
        ])
        .current_dir(&outside_workspace)
        .output()
        .expect("the foreign find starts");
    assert_success(&found);
    let found = String::from_utf8(found.stdout).expect("plain output is UTF-8");
    let quoted_before = before.replace('\'', "'\"'\"'");
    assert!(found.contains(&format!(
        "cargo optic --optic-dir '{quoted_before}' show --instance"
    )));

    let shown = fixture
        .command([
            "--optic-dir",
            before,
            "show",
            "--instance",
            &instance_id,
            "--source",
            "--format",
            "jsonl",
        ])
        .current_dir(&outside_workspace)
        .output()
        .expect("the foreign show starts");
    assert_success(&shown);
    assert!(json(&shown)["result"]["source"]["text"].is_string());

    let compared = fixture
        .command([
            "compare",
            "--before",
            &instance_id,
            "--before-optic-dir",
            before,
            "--after",
            &instance_id,
            "--after-optic-dir",
            after,
            "--format",
            "jsonl",
        ])
        .current_dir(&outside_workspace)
        .output()
        .expect("the cross-store comparison starts");
    assert_success(&compared);
    assert_eq!(json(&compared)["result"]["delta"]["bytes"], 0);

    let mutation = fixture
        .command(["--optic-dir", before, "gc", "--format", "jsonl"])
        .current_dir(&outside_workspace)
        .output()
        .expect("the rejected mutation starts");
    assert!(!mutation.status.success());
    assert!(
        string(&json(&mutation), "/error/message")
            .contains("--optic-dir requires a read-only command")
    );
    assert_eq!(durable_paths_below(Path::new(before)), before_entries);
}

#[test]
fn captures_and_reports_optimization_remark_state() {
    let fixture = Fixture::new();
    let shown = fixture.run([
        "show",
        "optic_mvp_kernel::ReexportedKernel::identity",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--fresh",
        "--output",
        "remarks",
        "--limit",
        "1",
        "--format",
        "jsonl",
    ]);
    assert_success(&shown);
    let shown = json(&shown);
    let capture_id = string(&shown, "/result/capture_id");
    assert_ne!(shown["result"]["summary"]["state"], "not-captured");

    let instance_id = stored_instance_ids(&fixture, capture_id)
        .into_iter()
        .next()
        .expect("the remark capture contains an instance");
    let catalog = Connection::open(fixture.root.join(".optic/store/catalog.sqlite"))
        .expect("the test can open the evidence catalog");
    let captures_before = catalog
        .query_row("SELECT COUNT(*) FROM captures", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("the test can count captures");
    let stored = fixture.run([
        "show",
        "--instance",
        &instance_id,
        "--output",
        "remarks",
        "--limit",
        "1",
        "--format",
        "jsonl",
    ]);
    assert_success(&stored);
    let stored = json(&stored);
    assert_ne!(stored["result"]["summary"]["state"], "not-captured");
    assert!(stored["result"]["remarks"].is_array());
    let captures_after = catalog
        .query_row("SELECT COUNT(*) FROM captures", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("the test can recount captures");
    assert_eq!(captures_after, captures_before);

    let inspected = fixture.run(["inspect", "--capture", capture_id, "--format", "jsonl"]);
    assert_success(&inspected);
    assert!(json(&inspected)["result"]["remark_files"].is_array());
}

#[test]
fn invalid_remark_filters_do_not_capture() {
    let fixture = Fixture::new();
    let rejected = fixture.run([
        "show",
        "optic_mvp_kernel::outlined_sum",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--output",
        "remarks",
        "--pass",
        "",
        "--format",
        "jsonl",
    ]);

    assert!(!rejected.status.success());
    assert_eq!(
        jsonl_events(&rejected)
            .last()
            .expect("the rejected show has a terminal event")["command"],
        "show"
    );
    assert!(
        string(&json(&rejected), "/error/message")
            .contains("remark pass must not be empty, got an empty pass")
    );
    let catalog = Connection::open(fixture.root.join(".optic/store/catalog.sqlite"))
        .expect("the test can open the evidence catalog");
    let captures = catalog
        .query_row("SELECT COUNT(*) FROM captures", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("the test can count captures");

    assert_eq!(captures, 0);
    assert!(!fixture.root.join("target/release").exists());
}

#[test]
fn resumes_retained_ingestion_before_cargo_even_with_fresh() {
    let fixture = Fixture::new();
    let catalog = fixture.install_failing_capture_trigger();
    let failed = fixture.run(capture_fresh_arguments());
    assert!(!failed.status.success());
    assert!(string(&json(&failed), "/error/message").contains("test capture publication failure"));
    let retained = json(&fixture.run(["status", "--format", "jsonl"]));
    assert_eq!(retained["result"]["pending"], 1);
    let pending = json(&fixture.run(["pending", "--format", "jsonl"]));
    let pending_id = string(&pending, "/result/0/id");
    assert!(pending_id.starts_with("pen_"));
    let inspected = json(&fixture.run(["pending", "inspect", pending_id, "--format", "jsonl"]));
    assert_eq!(string(&inspected, "/result/id"), pending_id);
    assert!(string(&inspected, "/result/capture_id").starts_with("cap_"));
    catalog
        .execute("DROP TRIGGER fail_capture_publication", [])
        .expect("the test can enable capture publication");

    let resumed = fixture.run(capture_fresh_arguments());
    assert_success(&resumed);
    assert_eq!(json(&resumed)["result"]["disposition"], "resumed");
    let completed = json(&fixture.run(["status", "--format", "jsonl"]));
    assert_eq!(completed["result"]["pending"], 0);
}

#[test]
fn changed_cargo_input_discards_retained_ingestion() {
    let fixture = Fixture::new();
    let catalog = fixture.install_failing_capture_trigger();
    let failed = fixture.run(capture_fresh_arguments());
    assert!(!failed.status.success());
    fs::write(
        fixture.root.join("kernel/src/build-data.txt"),
        "changed after compilation\n",
    )
    .expect("the test can change the included compiler input");
    catalog
        .execute("DROP TRIGGER fail_capture_publication", [])
        .expect("the test can enable capture publication");

    let captured = fixture.run(capture_fresh_arguments());
    assert_success(&captured);
    assert_eq!(json(&captured)["result"]["disposition"], "captured");
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
        "jsonl",
    ];
    let first = fixture.run(arguments);
    assert_success(&first);
    let first = json(&first);
    let first_id = string(&first, "/result/id");

    let reused = fixture.run(arguments);
    assert_success(&reused);
    let reused = json(&reused);
    assert_eq!(string(&reused, "/result/id"), first_id);
    assert_eq!(reused["result"]["disposition"], "reused");

    fs::write(fixture.root.join("kernel/src/build-data.txt"), "second\n")
        .expect("the test can change the included compiler input");
    let included = fixture.run(arguments);
    assert_success(&included);
    let included = json(&included);
    let included_id = string(&included, "/result/id");
    assert_ne!(included_id, first_id);
    assert_eq!(included["result"]["disposition"], "captured");

    let environment = fixture
        .command(arguments)
        .env("OPTIC_TEST_VALUE", "second")
        .output()
        .expect("the Cargo Optic binary starts");
    assert_success(&environment);
    let environment = json(&environment);
    assert_ne!(string(&environment, "/result/id"), included_id);
    assert_eq!(environment["result"]["disposition"], "captured");
}

#[test]
fn malformed_arguments_use_the_json_lines_error_contract() {
    for format in [
        ["--format", "jsonl"].as_slice(),
        ["--format=jsonl"].as_slice(),
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
        assert_eq!(output["version"], 1);
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
        "jsonl",
    ]);

    assert!(!output.status.success());
    assert_eq!(
        jsonl_events(&output)
            .last()
            .expect("the rejected capture has a terminal event")["command"],
        "capture"
    );
    assert!(
        string(&json(&output), "/error/message")
            .contains("--rustc-arg requires --evidence-profile experiment")
    );
}

#[cfg(unix)]
#[test]
fn closed_json_lines_output_stops_an_active_capture() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    let wrapper = fixture.root.join("slow-wrapper.sh");
    fs::write(
        &wrapper,
        concat!(
            "#!/bin/sh\n",
            "crate=\n",
            "previous=\n",
            "for argument do\n",
            "  if [ \"$previous\" = --crate-name ]; then crate=$argument; break; fi\n",
            "  previous=$argument\n",
            "done\n",
            "if [ \"$crate\" != optic_mvp_app ]; then exec \"$@\"; fi\n",
            "iteration=0\n",
            "while [ $iteration -lt 100 ]; do\n",
            "  printf 'slow wrapper is active\\n' >&2\n",
            "  sleep 0.1\n",
            "  iteration=$((iteration + 1))\n",
            "done\n",
            "exec \"$@\"\n",
        ),
    )
    .expect("the test can write the slow compiler wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))
        .expect("the test can make the slow compiler wrapper executable");
    let mut child = fixture
        .command([
            "capture",
            "-p",
            "optic-mvp-app",
            "--bin",
            "optic-mvp-app",
            "--release",
            "--fresh",
            "--format",
            "jsonl",
        ])
        .env("RUSTC_WRAPPER", &wrapper)
        .spawn()
        .expect("the cancellable capture starts");
    let stdout = child
        .stdout
        .take()
        .expect("the fixture command pipes standard output");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .expect("the test can read a JSON Lines event");
        assert_ne!(bytes, 0, "the capture reaches compiler execution");
        let event = serde_json::from_str::<Value>(&line).expect("the progress event is valid JSON");
        if event["data"]["message"] == "compiler capture started" {
            break;
        }
    }

    let cancellation_started = Instant::now();
    drop(reader);
    let output = child
        .wait_with_output()
        .expect("the cancelled capture is reaped");

    assert_success(&output);
    assert!(cancellation_started.elapsed() < Duration::from_secs(3));
    let status = json(&fixture.run(["status", "--format", "jsonl"]));
    assert_eq!(status["result"]["pending"], 0);
}

#[test]
fn captures_source_from_feature_selected_path_dependencies() {
    let fixture = Fixture::new();

    for feature_arguments in [
        &["--features", "optional-kernel"][..],
        &["--all-features"][..],
    ] {
        let mut arguments = vec![
            "show",
            "optic_mvp_optional_kernel::optional_source",
            "-p",
            "optic-mvp-app",
            "--bin",
            "optic-mvp-app",
            "--release",
            "--source",
            "--fresh",
            "--format",
            "jsonl",
        ];
        arguments.extend_from_slice(feature_arguments);
        let output = fixture
            .command(arguments)
            .output()
            .expect("the feature-selected capture starts");

        assert_success(&output);
        let result = json(&output);
        assert!(
            string(&result, "/result/source/text")
                .contains("pub fn optional_source<T>(value: T) -> T")
        );
        assert!(
            output
                .stdout
                .windows(b"streamed fixture warning".len())
                .any(|window| { window == b"streamed fixture warning" })
        );
    }
}

#[test]
fn comparison_reports_effective_rustflags() {
    let fixture = Fixture::new();
    let arguments = [
        "show",
        "optic_mvp_kernel::ReexportedKernel::identity",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--fresh",
        "--format",
        "jsonl",
    ];
    let before = fixture.run(arguments);
    assert_success(&before);
    let before = json(&before);
    let before_instance = string(&before, "/result/instance/id");
    let after = fixture
        .command(arguments)
        .env("RUSTFLAGS", "-C target-cpu=native")
        .output()
        .expect("the capture with RUSTFLAGS starts");
    assert_success(&after);
    let after = json(&after);
    let after_instance = string(&after, "/result/instance/id");
    let comparison = fixture.run([
        "compare",
        "--before",
        before_instance,
        "--after",
        after_instance,
        "--format",
        "jsonl",
    ]);
    assert_success(&comparison);
    let comparison = json(&comparison);
    let differences = comparison["result"]["compatibility_differences"]
        .as_array()
        .expect("the comparison includes compatibility dimensions");

    assert!(
        differences
            .iter()
            .any(|difference| difference == "compiler environment")
    );
    assert!(
        differences
            .iter()
            .any(|difference| difference == "rustc arguments")
    );
}

#[cfg(unix)]
#[test]
fn resolves_rustc_from_the_manifest_workspace() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let caller = fixture
        .temporary
        .path()
        .join("caller-outside-the-workspace");
    fs::create_dir(&caller).expect("the test can create the caller directory");
    let rustc = rustc_path();
    let rustc_proxy = fixture.root.join("workspace-rustc.sh");
    let log = fixture.root.join("rustc-cwd.log");
    fs::write(
        &rustc_proxy,
        format!(
            "#!/bin/sh\npwd >> \"$OPTIC_TEST_RUSTC_CWD_LOG\"\nexec \"{}\" \"$@\"\n",
            rustc.display()
        ),
    )
    .expect("the test can write the rustc proxy");
    fs::set_permissions(&rustc_proxy, fs::Permissions::from_mode(0o700))
        .expect("the test can make the rustc proxy executable");
    let manifest = fixture.root.join("Cargo.toml");
    let output = fixture
        .command([
            "capture",
            "--manifest-path",
            manifest.to_str().expect("the fixture path is UTF-8"),
            "-p",
            "optic-mvp-app",
            "--bin",
            "optic-mvp-app",
            "--release",
            "--fresh",
            "--format",
            "jsonl",
        ])
        .current_dir(&caller)
        .env("RUSTC", &rustc_proxy)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("OPTIC_TEST_RUSTC_CWD_LOG", &log)
        .output()
        .expect("the cross-workspace capture starts");

    assert_success(&output);
    let expected = fixture
        .root
        .canonicalize()
        .expect("the fixture root exists");
    let invocations = fs::read_to_string(log).expect("the rustc proxy log is readable");
    assert!(!invocations.is_empty());
    assert!(
        invocations
            .lines()
            .all(|directory| Path::new(directory) == expected)
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
        .env_remove("RUSTC_BOOTSTRAP")
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
            "jsonl",
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

#[cfg(unix)]
#[test]
fn scopes_unstable_access_to_the_selected_target() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let wrapper = fixture.root.join("bootstrap-wrapper.sh");
    let log = fixture.root.join("bootstrap-wrapper.log");
    fs::write(
        &wrapper,
        concat!(
            "#!/bin/sh\n",
            "crate=\n",
            "previous=\n",
            "for argument do\n",
            "  if [ \"$previous\" = --crate-name ]; then crate=$argument; break; fi\n",
            "  previous=$argument\n",
            "done\n",
            "if [ -n \"$crate\" ]; then printf '%s:%s\\n' \"$crate\" ",
            "\"${RUSTC_BOOTSTRAP-unset}\" >> \"$OPTIC_TEST_WRAPPER_LOG\"; fi\n",
            "exec \"$@\"\n",
        ),
    )
    .expect("the test can write the compiler wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))
        .expect("the test can make the compiler wrapper executable");

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
            "jsonl",
        ])
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTC_WRAPPER", &wrapper)
        .env("OPTIC_TEST_WRAPPER_LOG", &log)
        .output()
        .expect("the Cargo Optic capture starts");
    assert_success(&capture);
    let invocations = fs::read_to_string(&log).expect("the compiler wrapper log is readable");
    let mut saw_non_selected_invocation = false;
    let mut saw_selected_target = false;

    for invocation in invocations.lines() {
        let (crate_name, bootstrap) = invocation
            .split_once(':')
            .expect("the wrapper recorded a crate name and bootstrap state");

        if crate_name == "optic_mvp_app" {
            saw_selected_target = true;
            assert_eq!(bootstrap, "1");
        } else {
            saw_non_selected_invocation = true;
            assert_ne!(
                bootstrap, "1",
                "non-selected invocation received Optic bootstrap: {invocation}"
            );
        }
    }
    assert!(
        saw_non_selected_invocation,
        "the capture passed through a non-selected compiler invocation"
    );
    assert!(
        saw_selected_target,
        "the selected target passed through the wrapper"
    );
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_compiler_arguments_without_panicking() {
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::new();
    let capture = fixture.run(capture_fresh_arguments());
    assert_success(&capture);
    let capture = json(&capture);
    let details = fixture.run([
        "inspect",
        "--capture",
        string(&capture, "/result/id"),
        "--format",
        "jsonl",
    ]);
    assert_success(&details);
    let details = json(&details);
    let driver = string(&details, "/result/wrapper_chain/0");
    let invalid_argument = std::ffi::OsString::from_vec(vec![0xff]);
    let output = Command::new(driver)
        .arg("rustc")
        .arg(invalid_argument)
        .output()
        .expect("the rustc identity driver starts");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("requires UTF-8 compiler arguments, got a non-UTF-8 argument")
    );
}

#[cfg(unix)]
fn rustc_path() -> PathBuf {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc can report its sysroot");
    assert_success(&output);

    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("the sysroot is UTF-8")
            .trim(),
    )
    .join("bin/rustc")
}

struct Fixture {
    temporary: TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic");
        let root = temporary.path().join("generic");
        copy_tree(&source, &root);

        Self { temporary, root }
    }

    fn run<const N: usize>(&self, arguments: [&str; N]) -> Output {
        self.command(arguments)
            .output()
            .expect("the Cargo Optic binary starts")
    }

    fn command<I, S>(&self, arguments: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-optic"));
        command
            .arg("optic")
            .args(arguments)
            .current_dir(&self.root)
            .env_remove("RUSTC_BOOTSTRAP")
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

    fn install_failing_capture_trigger(&self) -> Connection {
        assert_success(&self.run(["status"]));
        let catalog = Connection::open(self.root.join(".optic/store/catalog.sqlite"))
            .expect("the test can open the Optic catalog");
        catalog
            .execute_batch(
                "CREATE TRIGGER fail_capture_publication
                 BEFORE INSERT ON captures
                 BEGIN
                     SELECT RAISE(FAIL, 'test capture publication failure');
                 END;",
            )
            .expect("the test can disable capture publication");

        catalog
    }

    fn work_is_empty(&self) -> bool {
        fs::read_dir(self.root.join(".optic/store/work"))
            .expect("the store contains a work directory")
            .next()
            .is_none()
    }
}

fn capture_fresh_arguments() -> [&'static str; 10] {
    [
        "capture",
        "-p",
        "optic-mvp-app",
        "--bin",
        "optic-mvp-app",
        "--release",
        "--fresh",
        "--format",
        "jsonl",
        "--locked",
    ]
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

#[track_caller]
fn copy_directory(source: &Path, destination: &Path) {
    for entry in WalkDir::new(source) {
        let entry = entry.expect("the source directory is readable");
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("copied entries remain below the source");
        let target = destination.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).expect("the test can create copied directories");
        } else {
            fs::copy(entry.path(), &target).expect("the test can copy a file");
        }
    }
}

fn durable_paths_below(root: &Path) -> Vec<PathBuf> {
    let mut paths = WalkDir::new(root)
        .into_iter()
        .map(|entry| {
            entry
                .expect("the directory inventory is readable")
                .path()
                .strip_prefix(root)
                .expect("inventory entries remain below their root")
                .to_owned()
        })
        .filter(|path| {
            !matches!(
                path.file_name().and_then(OsStr::to_str),
                Some("catalog.sqlite-shm" | "catalog.sqlite-wal")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths
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
    let events = jsonl_events(output);
    let terminal = events
        .last()
        .expect("JSON Lines output contains a terminal event");
    if terminal["event"] == "error" {
        return serde_json::json!({
            "version": terminal["version"],
            "ok": false,
            "error": terminal["data"],
        });
    }
    if terminal["command"] == "captures" {
        let captures = events
            .iter()
            .filter(|event| event["event"] == "item")
            .map(|event| event["data"].clone())
            .collect::<Vec<_>>();

        return serde_json::json!({
            "version": terminal["version"],
            "ok": true,
            "result": captures,
        });
    }
    if terminal["command"] == "inspect" {
        let started = events
            .iter()
            .find(|event| event["data"]["kind"] == "started")
            .expect("a streamed inspection starts with metadata");
        let mut result = started["data"]["metadata"].clone();
        let result = result
            .as_object_mut()
            .expect("inspection metadata is a JSON object");
        result.insert(
            "artifacts".to_owned(),
            Value::Array(
                events
                    .iter()
                    .filter(|event| event["data"]["kind"] == "artifact")
                    .map(|event| event["data"]["artifact"].clone())
                    .collect(),
            ),
        );
        result.insert(
            "remark_files".to_owned(),
            Value::Array(
                events
                    .iter()
                    .filter(|event| event["data"]["kind"] == "remark-file")
                    .map(|event| event["data"]["remark_file"].clone())
                    .collect(),
            ),
        );

        return serde_json::json!({
            "version": terminal["version"],
            "ok": true,
            "result": result,
        });
    }
    let streamed_show = events
        .iter()
        .any(|event| event["data"]["kind"] == "started");
    if terminal["command"] != "show" || !streamed_show {
        return serde_json::json!({
            "version": terminal["version"],
            "ok": true,
            "result": terminal["data"],
        });
    }

    collected_show(&events)
}

#[track_caller]
fn jsonl_events(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("each JSON Lines event is valid"))
        .collect()
}

#[track_caller]
fn assert_jsonl_stream(output: &Output) {
    assert!(
        output.stderr.is_empty(),
        "JSON Lines output must not use stderr, got:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let events = jsonl_events(output);
    assert!(!events.is_empty(), "a JSON Lines stream has events");

    for pair in events.windows(2) {
        let before = pair[0]["sequence"]
            .as_u64()
            .expect("an event has a sequence number");
        let after = pair[1]["sequence"]
            .as_u64()
            .expect("an event has a sequence number");
        assert!(before < after, "event sequence numbers increase");
    }

    let terminals = events
        .iter()
        .filter(|event| matches!(event["event"].as_str(), Some("complete" | "error")))
        .count();
    assert_eq!(terminals, 1, "a stream has one terminal event");
    assert!(
        matches!(
            events.last().and_then(|event| event["event"].as_str()),
            Some("complete" | "error")
        ),
        "the terminal event is last",
    );
}

fn collected_show(events: &[Value]) -> Value {
    let started = events
        .iter()
        .find(|event| event["data"]["kind"] == "started")
        .expect("a streamed show starts with metadata");
    let mut source = None;
    let mut source_text = String::new();
    let mut bodies = Vec::new();
    let mut body = None;

    for event in events {
        let data = &event["data"];
        match data["kind"].as_str() {
            Some("source-started") => {
                source = Some(serde_json::json!({
                    "path": data["path"],
                    "start_line": data["start_line"],
                    "text": "",
                }));
            }
            Some("source-chunk") => {
                source_text.push_str(data["text"].as_str().expect("a source chunk contains text"))
            }
            Some("body-started") => {
                body = Some(serde_json::json!({
                    "stage": data["stage"],
                    "module": data["module"],
                    "symbol": data["symbol"],
                    "text": "",
                    "summary": null,
                }));
            }
            Some("body-chunk") => {
                let body = body.as_mut().expect("a body chunk follows its header");
                let mut text = body["text"]
                    .as_str()
                    .expect("the collected body text is a string")
                    .to_owned();
                text.push_str(data["text"].as_str().expect("an LLVM chunk contains text"));
                body["text"] = Value::String(text);
            }
            Some("body-finished") => {
                let mut completed = body.take().expect("a body summary follows its body");
                completed["summary"] = data["summary"].clone();
                bodies.push(completed);
            }
            _ => {}
        }
    }
    if let Some(source) = &mut source {
        source["text"] = Value::String(source_text);
    }

    serde_json::json!({
        "version": started["version"],
        "ok": true,
        "result": {
            "capture_id": started["data"]["capture_id"],
            "instance": started["data"]["instance"],
            "output": started["data"]["output"],
            "source": source,
            "bodies": bodies,
        },
    })
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
    let output = fixture.run([
        "find",
        "--capture",
        capture_id,
        "optic_mvp",
        "--limit",
        "500",
        "--format",
        "jsonl",
    ]);
    assert_success(&output);
    let output = json(&output);

    output["result"]["instances"]
        .as_array()
        .expect("the fixture query returns instances")
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
