//! Runs public API scenarios in isolated test-executable children.
//!
//! The parent retains each fixture until the exact child test exits. API calls inherit only the
//! environment that the shared workspace applies before startup.

use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::run;

/// Returns the workspace in the child, or runs the child and returns nothing in the parent.
pub fn workspace_in_child(test: &str, fixture: impl AsRef<Path>) -> Option<PathBuf> {
    if env::var("OPTIC_TEST_CHILD").as_deref() == Ok(test) {
        return Some(env::current_dir().unwrap());
    }

    let workspace = TestWorkspace::new(fixture);
    let mut command = Command::new(env::current_exe().unwrap());
    workspace.apply(&mut command);
    command
        .args(["--exact", test, "--nocapture"])
        .env("OPTIC_TEST_CHILD", test);
    let output = run(&mut command);
    assert_success(&command, &output);

    None
}
