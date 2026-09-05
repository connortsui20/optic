//! Counts selected-target collection independently from dependency compilation.
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
use optic_compiler::collect_build;
use optic_compiler::discover_workspace;

fn collect_in_process(workspace: &TestWorkspace) {
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
        .env("OPTIC_TEST_CHILD", "1")
        .env("CARGO", workspace.observations().join("cargo"))
        .env("OPTIC_TEST_REAL_CARGO", cargo)
        .env(
            "OPTIC_TEST_OBSERVER",
            workspace.observations().join("rustc-observer"),
        )
        .env("OPTIC_TEST_EVENTS", workspace.observations().join("events"))
        .env("OPTIC_COMPILER_MODE", "collect");
    let output = run(&mut command);
    eprintln!(
        "observations: {}",
        fs::read_to_string(workspace.observations().join("events")).unwrap_or_default()
    );
    assert_success(&command, &output);
}

#[test]
fn counts_cold_collection_and_preserves_warm_dependencies() {
    let workspace = TestWorkspace::new("capture");

    for (name, source) in [
        ("cargo", include_str!("fixtures/observe-cargo.sh")),
        ("rustc-observer", include_str!("fixtures/observe-rustc.sh")),
    ] {
        let path = workspace.observations().join(name);
        fs::write(&path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    collect_in_process(&workspace);
    let cold = fs::read_to_string(workspace.observations().join("events")).unwrap();
    assert_eq!(
        cold.lines()
            .filter(|line| *line == "selected-collect")
            .count(),
        1,
        "{cold}"
    );
    assert_eq!(
        cold.lines().filter(|line| *line == "dependency").count(),
        1,
        "{cold}"
    );
    assert!(!cold.contains("selected-probe"), "{cold}");

    collect_in_process(&workspace);
    let warm = fs::read_to_string(workspace.observations().join("events")).unwrap();
    assert_eq!(
        warm.lines()
            .filter(|line| *line == "selected-collect")
            .count(),
        2,
        "{warm}"
    );
    assert_eq!(
        warm.lines().filter(|line| *line == "dependency").count(),
        1,
        "{warm}"
    );
    assert!(!warm.contains("selected-probe"), "{warm}");
}

#[test]
fn observed_collection_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("generic".to_owned()),
        "release",
    )
    .unwrap();
    let collection = collect_build(&workspace, &request).unwrap();
    let (_, _, instances) = collection.into_parts();
    assert!(!instances.is_empty());
}
