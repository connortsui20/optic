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

#[test]
fn retained_bitcode_proves_no_lto_and_local_thin_lto_stages() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let compiler = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    assert!(compiler.status.success());
    let sysroot = PathBuf::from(String::from_utf8(compiler.stdout).unwrap().trim());
    let rustc = sysroot.join("bin/rustc");
    let version = Command::new(&rustc).arg("-vV").output().unwrap();
    let version = String::from_utf8(version.stdout).unwrap();
    assert!(version.contains("commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985"));
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    let llvm_dis = sysroot.join("lib/rustlib").join(host).join("bin/llvm-dis");
    let driver = root.join("stage-proof");
    let source = root.join("stage-proof.rs");
    fs::write(&source, include_str!("../rustc-driver/stage-proof.rs")).unwrap();
    let mut compile = Command::new(&rustc);
    compile
        .args([
            "--crate-name",
            "optic_stage_proof",
            "--edition=2024",
            "-C",
            "prefer-dynamic",
            "-C",
            "rpath",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&driver)
        .env("RUSTC_BOOTSTRAP", "optic_stage_proof");
    let output = compile.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let input = root.join("input.rs");
    fs::write(&input, r#"
pub mod first {
    #[unsafe(no_mangle)]
    #[inline(never)]
    pub fn folded_first(value: u64) -> u64 { value.wrapping_add(7).wrapping_sub(value).wrapping_add(35) }
}
pub mod second {
    #[unsafe(no_mangle)]
    #[inline(never)]
    pub fn folded_second(value: u64) -> u64 { value.wrapping_add(7).wrapping_sub(value).wrapping_add(10) }
}
fn main() { assert_eq!(first::folded_first(10) + second::folded_second(20), 59); }
"#).unwrap();

    for (label, lto) in [("no-lto", Some("lto=off")), ("local-thin", None)] {
        let mut configurations = Vec::new();

        for retain in [false, true] {
            let directory = root.join(format!("{label}-{retain}"));
            fs::create_dir(&directory).unwrap();
            let executable = directory.join("proof-input");
            let mut command = Command::new(&driver);
            command
                .arg(&input)
                .args([
                    "--crate-name",
                    "proof_input",
                    "--edition=2024",
                    "-C",
                    "opt-level=3",
                    "-C",
                    "codegen-units=4",
                ])
                .arg("-o")
                .arg(&executable)
                .env("OPTIC_PROOF_DIRECTORY", &directory);
            if let Some(lto) = lto {
                command.arg("-C").arg(lto);
            }
            if retain {
                command.env("OPTIC_PROOF_RETAIN", "1");
            }

            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{command:?}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(Command::new(&executable).status().unwrap().success());
            configurations.push(fs::read_to_string(directory.join("configuration")).unwrap());
            if !retain {
                continue;
            }

            let modules = fs::read_to_string(directory.join("modules")).unwrap();
            assert!(modules.lines().count() > 1, "{modules}");
            let mut folded = 0;

            for module in modules.lines() {
                let (_, paths) = module.split_once('\t').unwrap();
                let (path, before) = paths.split_once('\t').unwrap();
                assert!(fs::metadata(path).unwrap().len() > 0, "{path}");
                let output = Command::new(&llvm_dis)
                    .args([path, "-o", "-"])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let text = String::from_utf8(output.stdout).unwrap();

                for (symbol, expected) in [
                    ("folded_first", "ret i64 42"),
                    ("folded_second", "ret i64 17"),
                ] {
                    if let Some(header) = text.lines().find(|line| {
                        line.starts_with("define ") && line.contains(&format!("@{symbol}("))
                    }) {
                        let start = text.find(header).unwrap();
                        let body = text[start..].split_once("\n}").unwrap().0;
                        assert!(body.contains(expected), "{path}: {body}");
                        let output = Command::new(&llvm_dis)
                            .args([before, "-o", "-"])
                            .output()
                            .unwrap();
                        assert!(output.status.success());
                        let before = String::from_utf8(output.stdout).unwrap();
                        let header = before
                            .lines()
                            .find(|line| {
                                line.starts_with("define ") && line.contains(&format!("@{symbol}("))
                            })
                            .unwrap();
                        let body = before[before.find(header).unwrap()..]
                            .split_once("\n}")
                            .unwrap()
                            .0;
                        assert!(
                            !body.contains(expected),
                            "the no-opt module must precede constant folding: {body}"
                        );
                        folded += 1;
                    }
                }
            }

            assert_eq!(folded, 2);
            eprintln!(
                "{label}: {} regular modules\n{}",
                modules.lines().count(),
                configurations.last().unwrap()
            );
        }

        assert_eq!(configurations[0], configurations[1]);
    }
}
