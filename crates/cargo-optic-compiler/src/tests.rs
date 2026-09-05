//! Exercises real driver provisioning and the shared private protocol in isolated children.
//!
//! Child environments own their Cargo home and temporary paths. The parent never mutates process
//! environment variables, and the provisioning counter exists only in compiler unit-test builds.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use optic_records::CompilerIdentity;

use crate::BuildRequest;
use crate::CargoTarget;
use crate::Error;
use crate::Freshness;
use crate::discover_workspace;
use crate::driver::RustcDriver;
use crate::prepare_build;
use crate::toolchain::CompilerContext;

pub(crate) fn artifact() -> cargo_metadata::Artifact {
    serde_json::from_value(serde_json::json!({
        "package_id": "path+file:///workspace#fixture@0.1.0",
        "manifest_path": "/workspace/Cargo.toml",
        "target": {
            "kind": ["lib"], "crate_types": ["lib"], "name": "fixture",
            "src_path": "/workspace/src/lib.rs", "edition": "2024",
            "doc": true, "doctest": true, "test": true
        },
        "profile": {"opt_level": "2", "debuginfo": 2, "debug_assertions": false, "overflow_checks": true, "test": false},
        "features": ["beta", "alpha"], "filenames": ["/target/libfixture.rlib"],
        "executable": null, "fresh": true
    })).unwrap()
}

#[track_caller]
fn child(fixture: &TestWorkspace, test: &str) -> std::process::Output {
    let mut command = Command::new(env::current_exe().unwrap());
    fixture.apply(&mut command);
    command
        .args(["--exact", test, "--nocapture"])
        .env("OPTIC_TEST_CHILD", "1")
        .env(
            "OPTIC_TEST_DRIVER_BUILDS",
            fixture.observations().join("driver-builds"),
        );
    let output = cargo_optic_test_support::run(&mut command);
    cargo_optic_test_support::assert_success(&command, &output);

    output
}

#[test]
fn driver_cache_reuses_actual_compilation_across_processes() {
    let fixture = TestWorkspace::new("capture");
    let count = fixture.observations().join("driver-builds");
    child(&fixture, "tests::driver_cache_child");
    assert_eq!(fs::read_to_string(&count).unwrap(), "build\n");
    child(&fixture, "tests::driver_cache_child");
    assert_eq!(fs::read_to_string(&count).unwrap(), "build\n");
}

#[test]
fn driver_cache_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let compiler = CompilerContext::discover(&workspace).unwrap();
    RustcDriver::provision(&workspace, compiler.identity()).unwrap();
}

#[test]
fn failed_driver_build_reports_components_without_publishing_a_cache_entry() {
    let fixture = TestWorkspace::new("capture");
    child(&fixture, "tests::failed_driver_build_child");

    assert_eq!(
        fs::read_to_string(fixture.observations().join("driver-builds")).unwrap(),
        "build\n"
    );
    assert_eq!(
        fs::read_dir(fixture.cargo_home().join("optic/drivers"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn failed_driver_build_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let compiler = CompilerContext::discover(&workspace).unwrap();
    let executable = PathBuf::from(env::var_os("TMPDIR").unwrap()).join("failed-rustc");
    fs::write(
        &executable,
        r#"#!/bin/sh
set -eu
previous=
for argument in "$@"; do
    if [ "$previous" = -o ]; then
        printf '%s\n' incomplete-driver > "$argument"
    fi
    previous=$argument
done
printf '%s\n' 'error[E0463]: rustc_driver is unavailable in this sysroot' >&2
exit 23
"#,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let identity = compiler.identity();
    let failing = CompilerIdentity::new(
        executable.clone(),
        identity.release(),
        identity.commit_hash(),
        identity.host(),
        identity.sysroot().to_owned(),
    )
    .unwrap();

    let Err(Error::ProcessFailed {
        program,
        status,
        diagnostics,
    }) = RustcDriver::provision(&workspace, &failing)
    else {
        panic!("the compiler shim must return its driver-build failure");
    };
    assert_eq!(program, executable);
    assert_eq!(status, "exit status: 23");
    let diagnostics = diagnostics.expect("driver-build failures retain compiler diagnostics");
    assert!(
        diagnostics.contains("error[E0463]: rustc_driver is unavailable in this sysroot"),
        "{diagnostics}"
    );
    assert!(
        diagnostics.contains("rustup component add rustc-dev llvm-tools"),
        "{diagnostics}"
    );
}

#[test]
fn driver_protocol_stops_stale_probes_and_collects_new_tokens() {
    let output = child(
        &TestWorkspace::new("capture"),
        "tests::driver_protocol_child",
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("stopped a stale selected-target probe")
    );
}

#[test]
fn failed_retry_replays_probe_and_collection_diagnostics() {
    let output = child(&TestWorkspace::new("capture"), "tests::failed_retry_child");
    let diagnostics = String::from_utf8_lossy(&output.stderr);

    assert!(
        diagnostics.contains("stopped a stale selected-target probe"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("expected"), "{diagnostics}");
}

#[test]
fn failed_retry_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "release").unwrap();
    let (_, _, _, analysis) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts();
    fs::write(
        workspace.root().join("src/lib.rs"),
        "pub fn broken() { let = ; }\n",
    )
    .unwrap();
    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&analysis).unwrap(), Freshness::Stale);
    assert!(prepared.collect().is_err());
}

#[test]
fn driver_protocol_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "checked").unwrap();
    let (_, _, instances, analysis) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts();
    assert!(
        instances
            .iter()
            .any(|instance| instance.display_name().contains("captured_value"))
    );
    assert_eq!(analysis.artifact().artifact().profile.opt_level, "1");
    assert!(analysis.artifact().artifact().profile.debug_assertions);

    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&analysis).unwrap(), Freshness::Fresh);

    fs::write(
        workspace.root().join("src/lib.rs"),
        "pub fn kernel(value: u64) -> u64 { value + 200 }\n",
    )
    .unwrap();
    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&analysis).unwrap(), Freshness::Stale);
    let (_, _, _, changed) = prepared.collect().unwrap().into_parts();
    assert_ne!(changed.token(), analysis.token());
    assert_eq!(changed.request_key(), analysis.request_key());
    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&changed).unwrap(), Freshness::Fresh);

    let (_, _, _, forced) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts();
    assert_ne!(forced.token(), changed.token());
    assert_eq!(
        fs::read_to_string(env::var_os("OPTIC_TEST_DRIVER_BUILDS").unwrap()).unwrap(),
        "build\n"
    );
}
