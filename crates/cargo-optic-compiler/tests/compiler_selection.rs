//! Exercises the supported compiler environment through real Cargo processes.
//!
//! Each scenario starts in a child with private Cargo configuration and build directories.

#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::diagnostics;
use cargo_optic_test_support::run;
use optic_compiler::BuildRequest;
use optic_compiler::CargoTarget;
use optic_compiler::Freshness;
use optic_compiler::Workspace;
use optic_compiler::discover_workspace;
use optic_compiler::prepare_build;
use optic_records::CargoTargetKind;

fn child_command(workspace: &TestWorkspace, scenario: &str) -> Command {
    let mut command = Command::new(env::current_exe().unwrap());
    workspace.apply(&mut command);
    command
        .args(["--exact", "collect_in_child", "--nocapture"])
        .env("OPTIC_TEST_WORKSPACE", workspace.workspace())
        .env("OPTIC_TEST_SCENARIO", scenario);

    command
}

#[test]
fn collects_with_the_default_rustc() {
    let workspace = TestWorkspace::new("capture");
    let mut command = child_command(&workspace, "success");
    command
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "");
    let output = run(&mut command);

    assert_success(&command, &output);
}

#[test]
fn collects_a_warm_target_again() {
    let workspace = TestWorkspace::new("capture");
    let mut command = child_command(&workspace, "warm");
    command
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "");
    let output = run(&mut command);

    assert_success(&command, &output);
}

#[test]
fn disables_a_configured_wrapper_with_a_warning() {
    let workspace = TestWorkspace::new("capture");
    fs::create_dir(workspace.workspace().join(".cargo")).unwrap();
    let wrapper = workspace.workspace().join("wrapper");
    fs::write(
        &wrapper,
        "#!/bin/sh\ntouch \"$OPTIC_TEST_WORKSPACE/wrapper-ran\"\nexec \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        workspace.workspace().join(".cargo/config.toml"),
        "[build]\nrustc-wrapper = \"./wrapper\"\n",
    )
    .unwrap();

    let mut command = child_command(&workspace, "success");
    let output = run(&mut command);
    let diagnostics = diagnostics(&command, &output);

    assert!(output.status.success(), "{diagnostics}");
    assert!(!workspace.workspace().join("wrapper-ran").exists());
    assert!(diagnostics.contains(
        "warning: Cargo Optic does not support configured rustc wrappers; disabling them for this capture"
    ));
    assert!(
        diagnostics.contains(
            "warning: the captured compiler output can differ from a normal wrapped build"
        )
    );
}

#[test]
fn rejects_a_configured_compiler() {
    let workspace = TestWorkspace::new("capture");
    fs::create_dir(workspace.workspace().join(".cargo")).unwrap();
    fs::write(
        workspace.workspace().join(".cargo/config.toml"),
        "[build]\nrustc = \"rustc\"\n",
    )
    .unwrap();
    let mut command = child_command(&workspace, "compiler-error");
    command
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "");
    let output = run(&mut command);

    assert_success(&command, &output);
}

#[test]
fn rejects_an_environment_compiler() {
    let workspace = TestWorkspace::new("capture");
    let mut command = child_command(&workspace, "compiler-error");
    command
        .env("RUSTC", "rustc")
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "");
    let output = run(&mut command);

    assert_success(&command, &output);
}

#[test]
fn collects_and_probes_named_targets_from_a_subdirectory() {
    let workspace = TestWorkspace::new("capture");
    let subdirectory = workspace.workspace().join("src/subdir");
    fs::create_dir(&subdirectory).unwrap();
    let subdirectory = fs::canonicalize(subdirectory).unwrap();
    let mut command = child_command(&workspace, "named-targets");
    command
        .current_dir(&subdirectory)
        .env("OPTIC_TEST_WORKSPACE", &subdirectory);
    let output = run(&mut command);

    assert_success(&command, &output);
}

#[track_caller]
fn collect_and_probe_named_targets(workspace: &Workspace) {
    let invocation_directory = fs::canonicalize(workspace.root().join("src/subdir")).unwrap();
    assert_eq!(env::current_dir().unwrap(), invocation_directory);

    for (target, kind, definition) in [
        (
            CargoTarget::Example("selected_example".to_owned()),
            CargoTargetKind::Example,
            "selected_example::example_instance",
        ), // Explicit example selection.
        (
            CargoTarget::Benchmark("selected_benchmark".to_owned()),
            CargoTargetKind::Bench,
            "selected_benchmark::benchmark_instance",
        ), // Explicit benchmark selection without the test harness.
    ] {
        let request = BuildRequest::new("capture_fixture", target, "release").unwrap();
        let collected = prepare_build(workspace, &request)
            .unwrap()
            .collect()
            .unwrap();
        let (build, _, instances, analysis) = collected.into_parts();
        let (crate_name, _) = definition.split_once("::").unwrap();

        assert_eq!(build.package(), "capture_fixture");
        assert_eq!(build.target().kind(), kind);
        assert_eq!(build.target().name(), crate_name);
        assert_eq!(build.invocation_directory(), invocation_directory);

        let selected = instances
            .iter()
            .filter(|instance| instance.definition().definition_path() == definition)
            .collect::<Vec<_>>();
        assert_eq!(
            selected.len(),
            1,
            "{target:?}: {instances:?}",
            target = request.target()
        );
        assert_eq!(selected[0].definition().crate_name(), crate_name);
        assert!(!selected[0].raw_symbol().is_empty());

        let mut warm = prepare_build(workspace, &request).unwrap();
        assert_eq!(warm.request_key(), analysis.request_key());
        assert_eq!(warm.probe(&analysis).unwrap(), Freshness::Fresh);
    }
}

#[test]
fn collect_in_child() {
    let Some(directory) = env::var_os("OPTIC_TEST_WORKSPACE") else {
        return;
    };
    let workspace = discover_workspace(Path::new(&directory)).unwrap();

    if env::var("OPTIC_TEST_SCENARIO").unwrap() == "named-targets" {
        collect_and_probe_named_targets(&workspace);

        return;
    }

    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "release").unwrap();
    let result = prepare_build(&workspace, &request).and_then(|prepared| prepared.collect());

    match env::var("OPTIC_TEST_SCENARIO").unwrap().as_str() {
        "success" => {
            result.expect("collection with the default compiler must succeed");
        }
        "warm" => {
            result.expect("the first collection must succeed");
            prepare_build(&workspace, &request)
                .unwrap()
                .collect()
                .expect("the warm collection must run rustc again");
        }
        "compiler-error" => {
            let Err(error) = result else {
                panic!("custom compiler selection must fail");
            };
            assert!(
                error
                    .to_string()
                    .contains("custom rustc selection is not supported")
            );
        }
        scenario => panic!("unknown test scenario {scenario}"),
    }
}
