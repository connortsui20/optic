use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use optic_records::BuildRecord;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::CargoTargetKind;
use optic_records::TargetRecord;

use crate::Error;
use crate::RECORD_FILE_NAME;
use crate::Store;

fn record(id: &str, completed_at_unix_ms: u64) -> CaptureRecord {
    let target =
        TargetRecord::new("example", CargoTargetKind::Lib).expect("the fixture target is valid");
    let build = BuildRecord::new(
        "example",
        "0.1.0",
        target,
        "release",
        PathBuf::from("cargo"),
        std::env::current_dir().expect("the test invocation directory is available"),
        vec!["rustc".to_owned()],
    )
    .expect("the fixture build is valid");

    CaptureRecord::new(
        id.parse().expect("the fixture capture ID is valid"),
        completed_at_unix_ms,
        build,
    )
}

fn write_completed_record(store: &Store, directory_id: &str, capture: &CaptureRecord) -> PathBuf {
    let directory = store.root.join("captures").join(directory_id);
    fs::create_dir_all(&directory).expect("the completed capture directory can be created");
    let path = directory.join(RECORD_FILE_NAME);
    let encoded = serde_json::to_vec(capture).expect("the fixture record can be encoded");
    fs::write(&path, encoded).expect("the fixture record can be written");

    path
}

#[test]
fn lists_records_by_descending_recorded_completion_time() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);

    store
        .publish(&older)
        .expect("the older capture can be published");
    store
        .publish(&newer)
        .expect("the newer capture can be published");

    assert_eq!(
        store.captures().expect("captures can be listed"),
        vec![newer, older]
    );
}

#[test]
fn breaks_timestamp_ties_with_the_capture_id() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let larger_id = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let smaller_id = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 1_000);

    store
        .publish(&larger_id)
        .expect("the larger capture ID can be published");
    store
        .publish(&smaller_id)
        .expect("the smaller capture ID can be published");

    assert_eq!(
        store.captures().expect("captures can be listed"),
        vec![smaller_id, larger_id]
    );
}

#[test]
fn rejects_a_duplicate_capture_id() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    store
        .publish(&capture)
        .expect("the capture can be published once");

    let error = store
        .publish(&capture)
        .expect_err("the duplicate capture ID must be rejected");

    assert!(matches!(
        error,
        Error::CaptureExists { id } if id == *capture.id()
    ));
}

#[test]
fn ignores_staged_records() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let staged = store.root.join("staging/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy");
    fs::create_dir_all(&staged).expect("the staging directory can be created");
    fs::write(staged.join(RECORD_FILE_NAME), b"not complete")
        .expect("the staging record can be written");

    assert!(store.captures().expect("staging is not visible").is_empty());
}

#[test]
fn rejects_invalid_completed_json() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let completed_id = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx";
    let completed = store.root.join("captures").join(completed_id);
    fs::create_dir_all(&completed).expect("the completed directory can be created");
    let path = completed.join(RECORD_FILE_NAME);
    fs::write(&path, b"not JSON").expect("the invalid record can be written");

    let error = store
        .captures()
        .expect_err("the invalid completed JSON must be rejected");

    assert!(matches!(error, Error::Json { path: error_path, .. } if error_path == path));
}

#[test]
fn rejects_a_completed_directory_without_a_record() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let directory = store.root.join("captures/zyxwvutsrqponmlkzyxwvutsrqponmlk");
    fs::create_dir_all(&directory).expect("the completed capture directory can be created");
    let path = directory.join(RECORD_FILE_NAME);

    let error = store
        .captures()
        .expect_err("the missing completed record must be rejected");

    assert!(matches!(
        error,
        Error::Filesystem {
            operation: "open",
            path: error_path,
            source,
        } if error_path == path && source.kind() == ErrorKind::NotFound
    ));
}

#[test]
fn rejects_non_directory_entries_in_completed_history() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let captures = store.root.join("captures");
    fs::create_dir_all(&captures).expect("the completed namespace can be created");
    let path = captures.join("unexpected-file");
    fs::write(&path, b"not a capture directory").expect("the invalid entry can be written");

    let error = store
        .captures()
        .expect_err("the non-directory entry must be rejected");

    assert!(matches!(
        error,
        Error::ExpectedCaptureDirectory { path: error_path } if error_path == path
    ));
}

#[test]
fn rejects_noncanonical_capture_directory_names() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let name = "not-a-capture-id";
    let path = store.root.join("captures").join(name);
    fs::create_dir_all(&path).expect("the invalid capture directory can be created");

    let error = store
        .captures()
        .expect_err("the invalid capture directory name must be rejected");

    assert!(matches!(
        error,
        Error::InvalidCaptureDirectoryId {
            path: error_path,
            name: error_name,
            ..
        } if error_path == path && error_name == name
    ));
}

#[test]
fn rejects_a_record_id_that_differs_from_its_directory_id() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let directory_id: CaptureId = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy"
        .parse()
        .expect("the directory capture ID is valid");
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 1_000);
    let path = write_completed_record(&store, directory_id.as_str(), &capture);

    let error = store
        .captures()
        .expect_err("the mismatched record ID must be rejected");

    assert!(matches!(
        error,
        Error::MismatchedCaptureId {
            path: error_path,
            directory_id: error_directory_id,
            record_id,
        } if error_path == path
            && error_directory_id == directory_id
            && record_id == *capture.id()
    ));
}

#[test]
fn rejects_a_relative_workspace_root() {
    let error = Store::new(PathBuf::from("workspace").as_path())
        .err()
        .expect("the relative workspace root must be rejected");

    assert!(
        matches!(error, Error::WorkspaceRootNotAbsolute { path } if path == Path::new("workspace"))
    );
}
