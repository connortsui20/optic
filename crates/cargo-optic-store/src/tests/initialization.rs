//! Verifies that Optic state is ignored before builds can observe it.
//!
//! Initialization preserves conflicting user configuration and does not rewrite an identical file.

use std::fs;
use std::process::Command;
use std::time::Duration;
use std::time::SystemTime;

use crate::Error;
use crate::Store;

#[test]
fn initializes_ignored_state_without_rewriting_an_identical_ignore_file() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    assert!(!store.root.exists());
    store.initialize().unwrap();
    let ignore = temporary.path().join(".optic/.gitignore");
    assert_eq!(fs::read(&ignore).unwrap(), b"*\n");
    for namespace in ["staging", "captures", "candidates"] {
        assert!(store.root.join(namespace).is_dir());
    }

    let old_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    fs::File::options()
        .write(true)
        .open(&ignore)
        .unwrap()
        .set_modified(old_time)
        .unwrap();
    store.initialize().unwrap();
    assert_eq!(fs::metadata(ignore).unwrap().modified().unwrap(), old_time);
}

#[test]
fn preserves_conflicting_ignore_contents() {
    for contents in [b"".as_slice(), b"*", b"custom\n", b"*\n# user rule\n"] {
        let temporary = tempfile::tempdir().unwrap();
        let optic = temporary.path().join(".optic");
        fs::create_dir(&optic).unwrap();
        let ignore = optic.join(".gitignore");
        fs::write(&ignore, contents).unwrap();
        let store = Store::new(temporary.path()).unwrap();

        assert!(matches!(
            store.initialize(),
            Err(Error::ConflictingIgnoreFile { .. })
        ));
        assert_eq!(fs::read(ignore).unwrap(), contents);
        assert!(!store.root.exists());
    }
}

#[test]
fn git_excludes_store_output_without_a_root_ignore_rule() {
    let temporary = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(temporary.path())
            .status()
            .unwrap()
            .success()
    );
    let store = Store::new(temporary.path()).unwrap();
    store.initialize().unwrap();
    let output = store.root.join("captures/output");
    fs::write(&output, b"capture evidence").unwrap();
    let result = Command::new("git")
        .arg("-C")
        .arg(temporary.path())
        .args([
            "check-ignore",
            "--no-index",
            "--verbose",
            ".optic/store/captures/output",
        ])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).starts_with(".optic/.gitignore:1:*"));
    assert!(!temporary.path().join(".gitignore").exists());
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_ignore_files_without_writing_their_targets() {
    let temporary = tempfile::tempdir().unwrap();
    let optic = temporary.path().join(".optic");
    fs::create_dir(&optic).unwrap();
    let target = temporary.path().join("user-ignore");
    fs::write(&target, b"*\n").unwrap();
    std::os::unix::fs::symlink(&target, optic.join(".gitignore")).unwrap();
    let store = Store::new(temporary.path()).unwrap();

    assert!(matches!(
        store.initialize(),
        Err(Error::UnexpectedFileType { .. })
    ));
    assert_eq!(fs::read(target).unwrap(), b"*\n");
    assert!(!store.root.exists());
}
