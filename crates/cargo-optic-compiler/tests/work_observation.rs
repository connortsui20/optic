//! Counts selected-target probes and collection independently from dependency compilation.
//!
//! A stable Cargo forwarding shim wraps the product driver without changing its arguments. This
//! observes the outer selected invocation even though the driver runs rustc analysis in-process.

#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::run;
use optic_compiler::BuildRequest;
use optic_compiler::CargoTarget;
use optic_compiler::Freshness;
use optic_compiler::discover_workspace;
use optic_compiler::prepare_build;
use optic_records::CaptureId;

#[track_caller]
fn run_in_process(workspace: &TestWorkspace, operation: &str) {
    let mut command = Command::new(env::current_exe().unwrap());
    workspace.apply(&mut command);

    let cargo = command
        .get_envs()
        .find(|(name, _)| *name == "CARGO")
        .unwrap()
        .1
        .unwrap()
        .to_owned();

    command
        .args(["--exact", "observed_collection_child", "--nocapture"])
        .env("OPTIC_TEST_CHILD", operation)
        .env("CARGO", workspace.observations().join("cargo"))
        .env("OPTIC_TEST_REAL_CARGO", cargo)
        .env(
            "OPTIC_TEST_OBSERVER",
            workspace.observations().join("rustc-observer"),
        )
        .env("OPTIC_TEST_EVENTS", workspace.observations().join("events"))
        .env(
            "OPTIC_TEST_ANALYSIS",
            workspace.observations().join("analysis.json"),
        );

    let output = run(&mut command);
    eprintln!(
        "observations: {}",
        fs::read_to_string(workspace.observations().join("events")).unwrap_or_default()
    );

    assert_success(&command, &output);
}

#[test]
fn counts_cold_warm_stale_and_forced_selected_work() {
    let workspace = TestWorkspace::new("capture");

    for (name, source) in [
        ("cargo", include_str!("fixtures/observe-cargo.sh")), // Forward Cargo commands.
        ("rustc-observer", include_str!("fixtures/observe-rustc.sh")), // Observe rustc invocations.
    ] {
        let path = workspace.observations().join(name);
        fs::write(&path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    run_in_process(&workspace, "collect");
    assert_counts(&workspace, 1, 0, 1);

    run_in_process(&workspace, "fresh");
    assert_counts(&workspace, 1, 0, 1);

    let source = workspace.workspace().join("src/generic.rs");
    let contents = fs::read_to_string(&source).unwrap();
    fs::write(source, contents.replace("outlined_kernel", "edited_kernel")).unwrap();

    run_in_process(&workspace, "stale");
    assert_counts(&workspace, 1, 1, 1);

    run_in_process(&workspace, "collect");
    assert_counts(&workspace, 2, 1, 1);

    run_in_process(&workspace, "fresh");
    assert_counts(&workspace, 2, 1, 1);

    run_in_process(&workspace, "collect");
    assert_counts(&workspace, 3, 1, 1);

    run_in_process(&workspace, "fresh");
    assert_counts(&workspace, 3, 1, 1);
}

#[test]
fn observed_collection_child() {
    let Ok(operation) = env::var("OPTIC_TEST_CHILD") else {
        return;
    };

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("generic".to_owned()),
        "release",
    )
    .unwrap();
    let mut prepared = prepare_build(&workspace, &request).unwrap();
    let analysis_path = env::var_os("OPTIC_TEST_ANALYSIS").unwrap();

    match operation.as_str() {
        "collect" => {
            let request_key = prepared.request_key().clone();
            let collection = prepared.collect().unwrap();
            let (_, _, analysis, manifest, _artifacts) =
                collection.into_parts(CaptureId::generate()).unwrap();

            assert!(!manifest.instances().is_empty());
            assert_eq!(analysis.request_key(), &request_key);

            fs::write(analysis_path, serde_json::to_vec(&analysis).unwrap()).unwrap();
        }
        "fresh" | "stale" => {
            let analysis = serde_json::from_slice(&fs::read(analysis_path).unwrap()).unwrap();
            let freshness = prepared.probe(&analysis).unwrap();

            match operation.as_str() {
                "fresh" => assert!(matches!(freshness, Freshness::Fresh)),
                "stale" => assert!(matches!(freshness, Freshness::Stale)),
                _ => unreachable!(),
            }
        }
        _ => panic!("unknown observation operation {operation}"),
    }
}

#[track_caller]
fn assert_counts(
    workspace: &TestWorkspace,
    collections: usize,
    probes: usize,
    dependencies: usize,
) {
    let events = fs::read_to_string(workspace.observations().join("events")).unwrap();

    assert_eq!(
        events
            .lines()
            .filter(|line| *line == "selected-collect")
            .count(),
        collections,
        "{events}"
    );
    assert_eq!(
        events
            .lines()
            .filter(|line| *line == "selected-probe")
            .count(),
        probes,
        "{events}"
    );
    assert_eq!(
        events.lines().filter(|line| *line == "dependency").count(),
        dependencies,
        "{events}"
    );
}
