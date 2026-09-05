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

fn capture(
    workspace: &TestWorkspace,
    policy: CapturePolicy,
    expected: &str,
    history: usize,
) -> String {
    let mut command = Command::new(env::current_exe().unwrap());
    workspace.apply(&mut command);
    command
        .args(["--exact", "capture_child", "--nocapture"])
        .env("OPTIC_TEST_CHILD", expected)
        .env("OPTIC_TEST_HISTORY", history.to_string())
        .env(
            "OPTIC_TEST_FRESH",
            if matches!(policy, CapturePolicy::Fresh) {
                "1"
            } else {
                "0"
            },
        );
    let output = run(&mut command);
    assert_success(&command, &output);
    let stdout = String::from_utf8(output.stdout).unwrap();

    stdout
        .lines()
        .find_map(|line| line.strip_prefix("capture: "))
        .unwrap()
        .to_owned()
}

/// Retains names, sizes, and modification times to detect writes during warm reuse.
fn files(directory: &Path) -> Vec<(std::path::PathBuf, u64, std::time::SystemTime)> {
    let mut files = Vec::new();

    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();

        if metadata.is_dir() {
            files.extend(self::files(&entry.path()));
        } else {
            files.push((entry.path(), metadata.len(), metadata.modified().unwrap()));
        }
    }

    files.sort();

    files
}

#[test]
fn reuses_edits_and_forces_capture_across_processes() {
    let workspace = TestWorkspace::new("capture");
    let first = capture(&workspace, CapturePolicy::Reuse, "captured", 1);
    let store_files = files(&workspace.workspace().join(".optic"));
    let artifacts = files(&workspace.target());
    let build_files = files(&workspace.build());

    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 1),
        first
    );
    assert_eq!(files(&workspace.workspace().join(".optic")), store_files);
    assert_eq!(files(&workspace.target()), artifacts);
    assert_eq!(files(&workspace.build()), build_files);

    let source = workspace.workspace().join("src/generic.rs");
    let contents = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        contents.replace("outlined_kernel", "edited_kernel"),
    )
    .unwrap();
    let edited = capture(&workspace, CapturePolicy::Reuse, "captured", 2);
    assert_ne!(
        first.split_whitespace().next(),
        edited.split_whitespace().next()
    );
    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 2),
        edited
    );

    let forced = capture(&workspace, CapturePolicy::Fresh, "captured", 3);
    assert_ne!(
        edited.split_whitespace().next(),
        forced.split_whitespace().next()
    );
    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 3),
        forced
    );
}

#[test]
fn invalidates_local_dependency_and_build_script_inputs() {
    let workspace = TestWorkspace::new("capture");
    let first = capture(&workspace, CapturePolicy::Reuse, "captured", 1);

    let dependency = workspace.workspace().join("dependency/src/lib.rs");
    let contents = fs::read_to_string(&dependency).unwrap();
    fs::write(dependency, contents.replace("42", "43")).unwrap();
    let changed_dependency = capture(&workspace, CapturePolicy::Reuse, "captured", 2);
    assert_ne!(first, changed_dependency);
    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 2),
        changed_dependency
    );

    fs::write(workspace.workspace().join("tracked.txt"), "second\n").unwrap();
    let changed_input = capture(&workspace, CapturePolicy::Reuse, "captured", 3);
    assert_ne!(changed_dependency, changed_input);
    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 3),
        changed_input
    );
}

#[test]
fn does_not_reuse_old_evidence_after_a_config_variant_returns() {
    let workspace = TestWorkspace::new("capture");
    fs::create_dir(workspace.workspace().join(".cargo")).unwrap();
    let config = workspace.workspace().join(".cargo/config.toml");
    fs::write(&config, "").unwrap();
    let first = capture(&workspace, CapturePolicy::Reuse, "captured", 1);

    fs::write(&config, "[build]\nrustflags = [\"--cfg=optic_variant\"]\n").unwrap();
    let variant = capture(&workspace, CapturePolicy::Reuse, "captured", 2);
    assert_ne!(first, variant);
    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 2),
        variant
    );

    fs::write(&config, "").unwrap();
    let restored = capture(&workspace, CapturePolicy::Reuse, "captured", 3);
    assert_ne!(
        first.split_whitespace().next(),
        restored.split_whitespace().next()
    );
    assert_eq!(
        capture(&workspace, CapturePolicy::Reuse, "reused", 3),
        restored
    );
}

#[test]
fn excludes_store_output_from_default_build_script_tracking() {
    for git in [false, true] {
        let workspace = TestWorkspace::new("default-tracking");
        assert!(!workspace.workspace().join(".gitignore").exists());

        if git {
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

        let first = capture(&workspace, CapturePolicy::Reuse, "captured", 1);
        assert_eq!(
            fs::read(workspace.workspace().join(".optic/.gitignore")).unwrap(),
            b"*\n"
        );
        assert_eq!(
            capture(&workspace, CapturePolicy::Reuse, "reused", 1),
            first,
            "git: {git}"
        );
        assert!(!workspace.workspace().join(".gitignore").exists());
    }
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

        if history.len() > 1 {
            let original = history.last().unwrap();
            assert!(
                !optic
                    .find(original.id(), "outlined_kernel", 100)
                    .unwrap()
                    .instances()
                    .is_empty()
            );
        }
    }

    println!(
        "capture: {} {}",
        capture.id(),
        capture.completed_at_unix_ms()
    );
}
