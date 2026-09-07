//! Exercises capture reuse through successive public API processes.
//!
//! Each child opens the same isolated workspace. Capture IDs and completion times supplement the
//! compiler tests that count selected collection and driver compilation.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::run;
use optic::BuildRequest;
use optic::CaptureOutcome;
use optic::CapturePolicy;
use optic::CargoTarget;
use optic::Optic;

#[track_caller]
fn capture_command(workspace: &TestWorkspace, policy: CapturePolicy) -> Command {
    let fresh = if matches!(policy, CapturePolicy::Fresh) {
        "1"
    } else {
        "0"
    };

    let mut command = Command::new(env::current_exe().unwrap());
    workspace.apply(&mut command);
    command
        .args(["--exact", "capture_child", "--nocapture"])
        .env("OPTIC_TEST_FRESH", fresh);

    command
}

/// Checks the child capture and history, then returns its ID and completion time as one line.
#[track_caller]
fn capture_and_check(
    command: &mut Command,
    expected_outcome: &str,
    expected_history_len: usize,
) -> String {
    command
        .env("OPTIC_TEST_CHILD", expected_outcome)
        .env("OPTIC_TEST_HISTORY", expected_history_len.to_string());

    let output = run(command);
    assert_success(command, &output);
    let stdout = String::from_utf8(output.stdout).unwrap();

    stdout
        .lines()
        .find_map(|line| line.strip_prefix("capture: "))
        .expect("capture_child prints the capture ID and completion time after its checks")
        .to_owned()
}

/// Records every file's path, size, and modification time in sorted order for warm-reuse checks.
#[track_caller]
fn snapshot_files(directory: &Path) -> Vec<(std::path::PathBuf, u64, std::time::SystemTime)> {
    let mut files = Vec::new();

    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();

        if metadata.is_dir() {
            files.extend(snapshot_files(&entry.path()));
        } else {
            files.push((entry.path(), metadata.len(), metadata.modified().unwrap()));
        }
    }

    files.sort();

    files
}

/// Retains the complete artifact inventory without Cargo bookkeeping modification times.
#[track_caller]
fn snapshot_file_sizes(directory: &Path) -> Vec<(std::path::PathBuf, u64)> {
    snapshot_files(directory)
        .into_iter()
        .map(|(path, size, _)| (path, size))
        .collect()
}

#[track_caller]
fn assert_stored_evidence(optic: &Optic, instance: &optic::FoundInstance) {
    let optic::SourceEvidence::Available { evidence, .. } =
        optic.source(instance.reference()).unwrap()
    else {
        panic!("expected captured generic source");
    };

    let mut source = Vec::new();
    optic.copy_evidence(&evidence, &mut source).unwrap();

    let function = instance
        .record()
        .definition()
        .definition_path()
        .rsplit("::")
        .next()
        .unwrap();

    assert!(
        String::from_utf8(source)
            .unwrap()
            .contains(&format!("fn {function}<"))
    );

    let optic::LlvmEvidence::Available(bodies) = optic.llvm(instance.reference()).unwrap() else {
        panic!("expected captured generic LLVM");
    };

    assert!(!bodies.is_empty());

    for body in bodies {
        let mut llvm = Vec::new();
        optic.copy_evidence(body.evidence(), &mut llvm).unwrap();

        assert!(!llvm.is_empty());
    }
}

/// Commits the fixture before testing Cargo's default build-script tracking in a Git repository.
#[track_caller]
fn commit_fixture_to_git(workspace: &TestWorkspace) {
    let mut command = Command::new("git");
    workspace.apply(&mut command);
    command.args(["init", "--quiet"]);
    let output = run(&mut command);
    assert_success(&command, &output);

    let mut command = Command::new("git");
    workspace.apply(&mut command);
    command.args(["add", "."]);
    let output = run(&mut command);
    assert_success(&command, &output);

    let mut command = Command::new("git");
    workspace.apply(&mut command);
    command.args([
        "-c",
        "user.name=Optic test",
        "-c",
        "user.email=optic-test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "--message",
        "Fixture",
    ]);
    let output = run(&mut command);
    assert_success(&command, &output);
}

#[test]
fn reuses_edits_and_forces_capture_across_processes() {
    let workspace = TestWorkspace::new("capture");
    let mut command = capture_command(&workspace, CapturePolicy::Reuse);
    let first = capture_and_check(&mut command, "captured", 1);
    let store_files = snapshot_files(&workspace.workspace().join(".optic"));
    let target_files = snapshot_file_sizes(&workspace.target());
    let build_files = snapshot_file_sizes(&workspace.build());
    let executable = workspace.target().join("release/generic");
    let executable_modified = fs::metadata(&executable).unwrap().modified().unwrap();

    assert_eq!(capture_and_check(&mut command, "reused", 1), first);
    assert_eq!(
        snapshot_files(&workspace.workspace().join(".optic")),
        store_files
    );
    assert_eq!(snapshot_file_sizes(&workspace.target()), target_files);
    assert_eq!(snapshot_file_sizes(&workspace.build()), build_files);
    assert_eq!(
        fs::metadata(executable).unwrap().modified().unwrap(),
        executable_modified
    );

    let source = workspace.workspace().join("src/generic.rs");
    let contents = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        contents.replace("outlined_kernel", "edited_kernel"),
    )
    .unwrap();

    let edited = capture_and_check(&mut command, "captured", 2);

    assert_ne!(
        first.split_whitespace().next(),
        edited.split_whitespace().next()
    );
    assert_eq!(capture_and_check(&mut command, "reused", 2), edited);

    let mut fresh = capture_command(&workspace, CapturePolicy::Fresh);
    let forced = capture_and_check(&mut fresh, "captured", 3);

    assert_ne!(
        edited.split_whitespace().next(),
        forced.split_whitespace().next()
    );
    assert_eq!(capture_and_check(&mut command, "reused", 3), forced);
}

#[test]
fn invalidates_local_dependency_and_build_script_inputs() {
    let workspace = TestWorkspace::new("capture");
    let mut command = capture_command(&workspace, CapturePolicy::Reuse);
    let first = capture_and_check(&mut command, "captured", 1);

    let dependency = workspace.workspace().join("dependency/src/lib.rs");
    let contents = fs::read_to_string(&dependency).unwrap();
    fs::write(dependency, contents.replace("42", "43")).unwrap();

    let changed_dependency = capture_and_check(&mut command, "captured", 2);

    assert_ne!(first, changed_dependency);
    assert_eq!(
        capture_and_check(&mut command, "reused", 2),
        changed_dependency
    );

    fs::write(workspace.workspace().join("tracked.txt"), "second\n").unwrap();

    let changed_input = capture_and_check(&mut command, "captured", 3);

    assert_ne!(changed_dependency, changed_input);
    assert_eq!(capture_and_check(&mut command, "reused", 3), changed_input);
}

#[test]
fn does_not_reuse_old_evidence_after_a_config_variant_returns() {
    let workspace = TestWorkspace::new("capture");
    let mut command = capture_command(&workspace, CapturePolicy::Reuse);

    fs::create_dir(workspace.workspace().join(".cargo")).unwrap();
    let config = workspace.workspace().join(".cargo/config.toml");
    fs::write(&config, "").unwrap();

    let first = capture_and_check(&mut command, "captured", 1);

    fs::write(&config, "[build]\nrustflags = [\"--cfg=optic_variant\"]\n").unwrap();

    let variant = capture_and_check(&mut command, "captured", 2);

    assert_ne!(first, variant);
    assert_eq!(capture_and_check(&mut command, "reused", 2), variant);

    fs::write(&config, "").unwrap();

    let restored = capture_and_check(&mut command, "captured", 3);

    assert_ne!(
        first.split_whitespace().next(),
        restored.split_whitespace().next()
    );
    assert_eq!(capture_and_check(&mut command, "reused", 3), restored);
}

#[test]
fn rejects_old_evidence_after_collection_succeeds_but_publication_fails() {
    let workspace = TestWorkspace::new("capture");
    let mut command = capture_command(&workspace, CapturePolicy::Reuse);
    let first = capture_and_check(&mut command, "captured", 1);
    let first_id = first.split_whitespace().next().unwrap();
    let completed = workspace.workspace().join(".optic/store/captures");
    let old_captures = snapshot_files(&completed);
    let candidates = fs::read_dir(workspace.workspace().join(".optic/store/candidates"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();

    assert_eq!(candidates.len(), 1);

    let pointer = &candidates[0];
    let old_pointer = fs::read(pointer).unwrap();
    let saved_pointer = workspace.observations().join("original-candidate.json");
    fs::rename(pointer, &saved_pointer).unwrap();
    fs::create_dir(pointer).unwrap();

    let source = workspace.workspace().join("src/generic.rs");
    let original = fs::read_to_string(&source).unwrap();
    assert!(original.contains("fn outlined_kernel"));
    assert!(original.contains("fn main() {"));

    let edited = original
        .replace("outlined_kernel", "edited_kernel")
        .replace(
            "fn main() {",
            "fn main() {\n    println!(\"edited fixture\");",
        );
    fs::write(&source, edited).unwrap();

    let mut failed = Command::new(env::current_exe().unwrap());
    workspace.apply(&mut failed);
    failed
        .args(["--exact", "publication_failure_child", "--nocapture"])
        .env("OPTIC_TEST_ORIGINAL_CAPTURE", first_id)
        .env("OPTIC_TEST_CANDIDATE", pointer);

    let output = run(&mut failed);
    assert_success(&failed, &output);

    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .any(|line| line == "publication rejected")
    );
    assert_eq!(snapshot_files(&completed), old_captures);
    assert!(pointer.is_dir());
    assert_eq!(fs::read(&saved_pointer).unwrap(), old_pointer);

    let mut executable = Command::new(workspace.target().join("release/generic"));
    workspace.apply(&mut executable);
    let output = run(&mut executable);
    assert_success(&executable, &output);
    assert_eq!(output.stdout, b"edited fixture\n");

    fs::remove_dir(pointer).unwrap();
    fs::rename(saved_pointer, pointer).unwrap();
    assert_eq!(fs::read(pointer).unwrap(), old_pointer);

    command.env("OPTIC_TEST_EDITED_EVIDENCE", "1");
    let next = capture_and_check(&mut command, "captured", 2);

    assert_ne!(next.split_whitespace().next().unwrap(), first_id);
    assert_eq!(capture_and_check(&mut command, "reused", 2), next);
}

#[test]
fn invalidates_build_script_environment_and_rustflags() {
    for (name, first_value, second_value) in [
        ("OPTIC_FIXTURE_VALUE", "first", "second"), // The script tracks this value.
        ("RUSTFLAGS", "--cfg=optic_first", "--cfg=optic_second"), // Cargo passes these flags.
    ] {
        let workspace = TestWorkspace::new("capture");
        let mut command = capture_command(&workspace, CapturePolicy::Reuse);
        command.env(name, first_value);
        let first = capture_and_check(&mut command, "captured", 1);

        assert_eq!(
            capture_and_check(&mut command, "reused", 1),
            first,
            "{name}"
        );

        command.env(name, second_value);
        let changed = capture_and_check(&mut command, "captured", 2);

        assert_ne!(
            first.split_whitespace().next(),
            changed.split_whitespace().next(),
            "{name}"
        );
        assert_eq!(
            capture_and_check(&mut command, "reused", 2),
            changed,
            "{name}"
        );

        command.env(name, first_value);
        let restored = capture_and_check(&mut command, "captured", 3);

        assert_ne!(
            first.split_whitespace().next(),
            restored.split_whitespace().next(),
            "{name}"
        );
        assert_ne!(
            changed.split_whitespace().next(),
            restored.split_whitespace().next(),
            "{name}"
        );
        assert_eq!(
            capture_and_check(&mut command, "reused", 3),
            restored,
            "{name}"
        );
    }
}

#[test]
fn excludes_store_output_from_default_build_script_tracking() {
    for git in [
        false, // Cargo discovers files without a Git repository.
        true,  // Cargo discovers files from a Git repository.
    ] {
        let workspace = TestWorkspace::new("default-tracking");
        let mut command = capture_command(&workspace, CapturePolicy::Reuse);

        assert!(!workspace.workspace().join(".gitignore").exists());

        if git {
            commit_fixture_to_git(&workspace);
        }

        let first = capture_and_check(&mut command, "captured", 1);

        assert_eq!(
            fs::read(workspace.workspace().join(".optic/.gitignore")).unwrap(),
            b"*\n"
        );
        assert_eq!(
            capture_and_check(&mut command, "reused", 1),
            first,
            "git: {git}"
        );
        assert!(!workspace.workspace().join(".gitignore").exists());
    }
}

#[test]
fn publication_failure_child() {
    let Ok(original) = env::var("OPTIC_TEST_ORIGINAL_CAPTURE") else {
        return;
    };

    let original = original.parse::<optic::CaptureId>().unwrap();
    let optic = Optic::open(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("generic".to_owned()),
        "release",
    )
    .unwrap();

    let error = optic
        .capture(&request, CapturePolicy::Fresh)
        .expect_err("the obstructed pointer must prevent publication");
    let optic::Error::Capture {
        source:
            optic::CaptureError::Store {
                source:
                    optic::StoreError::Filesystem {
                        operation, path, ..
                    },
            },
    } = error
    else {
        panic!("expected candidate replacement to fail after collection, got {error}");
    };

    assert_eq!(operation, "replace candidate");
    assert_eq!(
        path,
        std::path::PathBuf::from(env::var_os("OPTIC_TEST_CANDIDATE").unwrap())
    );

    let history = optic.list_captures().unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id(), &original);
    assert_eq!(
        optic
            .find(&original, "outlined_kernel", 100)
            .unwrap()
            .instances()
            .len(),
        2
    );
    assert!(
        optic
            .find(&original, "edited_kernel", 100)
            .unwrap()
            .instances()
            .is_empty()
    );

    println!("publication rejected");
}

#[test]
fn capture_child() {
    let Ok(expected) = env::var("OPTIC_TEST_CHILD") else {
        return;
    };

    let directory = env::current_dir().unwrap();
    let optic = Optic::open(&directory).unwrap();
    let default_tracking = !directory.join("src/generic.rs").exists();
    let request = if default_tracking {
        BuildRequest::new("default_tracking_fixture", CargoTarget::Library, "release")
    } else {
        BuildRequest::new(
            "capture_fixture",
            CargoTarget::Binary("generic".to_owned()),
            "release",
        )
    }
    .unwrap();

    let policy = if env::var("OPTIC_TEST_FRESH").unwrap() == "1" {
        CapturePolicy::Fresh
    } else {
        CapturePolicy::Reuse
    };

    let outcome = optic.capture(&request, policy).unwrap();

    assert_eq!(
        matches!(outcome, CaptureOutcome::Reused(_)),
        expected == "reused"
    );

    let capture = outcome.into_record();
    let history = optic.list_captures().unwrap();

    assert_eq!(
        history.len(),
        env::var("OPTIC_TEST_HISTORY")
            .unwrap()
            .parse::<usize>()
            .unwrap()
    );

    if !default_tracking {
        let found = optic.find(capture.id(), "kernel", 100).unwrap();
        assert!(!found.instances().is_empty());

        let generic = found
            .instances()
            .iter()
            .find(|instance| {
                let definition = instance.record().definition().definition_path();
                definition.ends_with("::outlined_kernel") || definition.ends_with("::edited_kernel")
            })
            .unwrap();

        assert_stored_evidence(&optic, generic);

        if env::var_os("OPTIC_TEST_EDITED_EVIDENCE").is_some() {
            assert_eq!(
                optic
                    .find(capture.id(), "edited_kernel", 100)
                    .unwrap()
                    .instances()
                    .len(),
                2
            );
            assert!(
                optic
                    .find(capture.id(), "outlined_kernel", 100)
                    .unwrap()
                    .instances()
                    .is_empty()
            );
        }

        if history.len() > 1 {
            let original = history.last().unwrap();
            let found = optic.find(original.id(), "outlined_kernel", 100).unwrap();

            assert!(!found.instances().is_empty());
            assert_stored_evidence(&optic, &found.instances()[0]);
        }
    }

    println!(
        "capture: {} {}",
        capture.id(),
        capture.completed_at_unix_ms()
    );
}
