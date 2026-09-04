//! Protects capture publication and completed-history reads.
//!
//! Valid and corrupt layouts cover staging, visibility, identity, and decoding failures.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use optic_records::BuildRecord;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::CargoTargetKind;
use optic_records::CompilerIdentity;
use optic_records::InstanceManifest;
use optic_records::TargetRecord;

use crate::CAPTURE_FILE_NAME;
use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::Store;

fn record(id: &str, completed_at_unix_ms: u64) -> CaptureRecord {
    record_with_counts(id, completed_at_unix_ms, 0, 0)
}

fn record_with_counts(
    id: &str,
    completed_at_unix_ms: u64,
    instance_count: u64,
    placement_count: u64,
) -> CaptureRecord {
    let invocation_directory =
        std::env::current_dir().expect("the test invocation directory is available");
    let target =
        TargetRecord::new("example", CargoTargetKind::Lib).expect("the fixture target is valid");
    let build = BuildRecord::new(
        "example",
        "0.1.0",
        target,
        "release",
        PathBuf::from("cargo"),
        invocation_directory.clone(),
        vec!["rustc".to_owned()],
    )
    .expect("the fixture build is valid");

    let compiler = CompilerIdentity::new(
        invocation_directory
            .join("toolchain")
            .join("bin")
            .join("rustc"),
        "1.99.0-nightly",
        "0123456789abcdef0123456789abcdef01234567",
        "x86_64-unknown-linux-gnu",
        invocation_directory.join("toolchain"),
    )
    .expect("the fixture compiler identity is valid");

    CaptureRecord::new(
        id.parse().expect("the fixture capture ID is valid"),
        completed_at_unix_ms,
        build,
        compiler,
        instance_count,
        placement_count,
    )
    .expect("the fixture capture record is valid")
}

fn manifest(id: &CaptureId) -> InstanceManifest {
    InstanceManifest::new(id.clone(), Vec::new()).expect("the fixture instance manifest is valid")
}

fn publish_capture(store: &Store, capture: &CaptureRecord) {
    store
        .publish(capture, &manifest(capture.id()))
        .expect("the capture can be published");
}

fn write_completed_record(store: &Store, directory_id: &str, capture: &CaptureRecord) -> PathBuf {
    let directory = store.root.join("captures").join(directory_id);
    fs::create_dir_all(&directory).expect("the completed capture directory can be created");
    let path = directory.join(CAPTURE_FILE_NAME);
    let encoded = serde_json::to_vec(capture).expect("the fixture record can be encoded");
    fs::write(&path, encoded).expect("the fixture record can be written");

    path
}

fn write_completed_manifest(
    store: &Store,
    directory_id: &str,
    instances: &InstanceManifest,
) -> PathBuf {
    let directory = store.root.join("captures").join(directory_id);
    fs::create_dir_all(&directory).expect("the completed capture directory can be created");
    let path = directory.join(INSTANCES_FILE_NAME);
    let encoded = serde_json::to_vec(instances).expect("the fixture manifest can be encoded");
    fs::write(&path, encoded).expect("the fixture manifest can be written");

    path
}

#[test]
fn publishes_and_reads_the_capture_and_instances_together() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    let instances = manifest(capture.id());

    store
        .publish(&capture, &instances)
        .expect("the complete capture can be published");

    let completed = store.root.join("captures").join(capture.id().as_str());
    assert!(completed.join(CAPTURE_FILE_NAME).is_file());
    assert!(completed.join(INSTANCES_FILE_NAME).is_file());
    assert_eq!(
        store
            .read_capture(capture.id())
            .expect("the capture record can be read"),
        capture
    );
    assert_eq!(
        store
            .read_instances(capture.id())
            .expect("the instance manifest can be read"),
        instances
    );
}

#[test]
fn lists_records_by_descending_recorded_completion_time() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);

    publish_capture(&store, &older);
    publish_capture(&store, &newer);

    assert_eq!(
        store.list_captures().expect("captures can be listed"),
        vec![newer, older]
    );
}

#[test]
fn breaks_timestamp_ties_with_the_capture_id() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let larger_id = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let smaller_id = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 1_000);

    publish_capture(&store, &larger_id);
    publish_capture(&store, &smaller_id);

    assert_eq!(
        store.list_captures().expect("captures can be listed"),
        vec![smaller_id, larger_id]
    );
}

#[test]
fn rejects_a_duplicate_capture_id() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    publish_capture(&store, &capture);

    let error = store
        .publish(&capture, &manifest(capture.id()))
        .expect_err("the duplicate capture ID must be rejected");

    assert!(matches!(
        error,
        Error::CaptureExists { id } if id == *capture.id()
    ));
}

#[test]
fn reports_a_missing_capture_without_exposing_the_store_layout() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let id: CaptureId = "zyxwvutsrqponmlkzyxwvutsrqponmlk"
        .parse()
        .expect("the fixture capture ID is valid");

    let error = store
        .read_instances(&id)
        .expect_err("the missing capture must be reported");

    assert!(matches!(&error, Error::CaptureNotFound { id: missing } if missing == &id));
    assert_eq!(
        error.to_string(),
        "completed capture does not exist, got zyxwvutsrqponmlkzyxwvutsrqponmlk"
    );
}

#[test]
fn rejects_records_for_different_captures_before_staging() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let manifest_id: CaptureId = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx"
        .parse()
        .expect("the manifest capture ID is valid");
    let instances = manifest(&manifest_id);

    let error = store
        .publish(&capture, &instances)
        .expect_err("records for different captures must be rejected");

    assert!(matches!(
        error,
        Error::MismatchedPublishedCaptureId {
            capture_id,
            manifest_id: error_manifest_id,
        } if capture_id == *capture.id() && error_manifest_id == manifest_id
    ));
    assert!(!store.root.exists());
}

#[test]
fn rejects_mismatched_counts_before_staging() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record_with_counts("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000, 1, 1);
    let instances = manifest(capture.id());

    let error = store
        .publish(&capture, &instances)
        .expect_err("mismatched record counts must be rejected");

    assert!(matches!(
        error,
        Error::MismatchedInstanceCounts {
            capture_id,
            capture_instance_count: 1,
            capture_placement_count: 1,
            manifest_instance_count: 0,
            manifest_placement_count: 0,
        } if capture_id == *capture.id()
    ));
    assert!(!store.root.exists());
}

#[test]
fn ignores_staged_records() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let staged = store.root.join("staging/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy");
    fs::create_dir_all(&staged).expect("the staging directory can be created");
    fs::write(staged.join(CAPTURE_FILE_NAME), b"not complete")
        .expect("the staging record can be written");

    assert!(
        store
            .list_captures()
            .expect("staging is not visible")
            .is_empty()
    );
}

#[test]
fn rejects_invalid_completed_json() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let completed_id = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx";
    let completed = store.root.join("captures").join(completed_id);
    fs::create_dir_all(&completed).expect("the completed directory can be created");
    let path = completed.join(CAPTURE_FILE_NAME);
    fs::write(&path, b"not JSON").expect("the invalid record can be written");

    let error = store
        .list_captures()
        .expect_err("the invalid completed JSON must be rejected");

    assert!(matches!(error, Error::Json { path: error_path, .. } if error_path == path));
}

#[test]
fn lists_and_reads_a_capture_without_parsing_its_invalid_instance_json() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 1_000);
    write_completed_record(&store, capture.id().as_str(), &capture);
    let path = store
        .root
        .join("captures")
        .join(capture.id().as_str())
        .join(INSTANCES_FILE_NAME);
    fs::write(&path, b"not JSON").expect("the invalid manifest can be written");

    assert_eq!(
        store.list_captures().expect("captures can be listed"),
        vec![capture.clone()]
    );
    assert_eq!(
        store
            .read_capture(capture.id())
            .expect("the capture header can be read"),
        capture
    );

    let error = store
        .read_instances(capture.id())
        .expect_err("invalid instance JSON must be rejected when evidence is read");

    assert!(matches!(error, Error::Json { path: error_path, .. } if error_path == path));
}

#[test]
fn rejects_a_completed_directory_without_a_record() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let directory = store.root.join("captures/zyxwvutsrqponmlkzyxwvutsrqponmlk");
    fs::create_dir_all(&directory).expect("the completed capture directory can be created");
    let path = directory.join(CAPTURE_FILE_NAME);

    let error = store
        .list_captures()
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
fn rejects_a_completed_directory_whose_instance_manifest_is_not_a_file() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    write_completed_record(&store, capture.id().as_str(), &capture);
    let path = store
        .root
        .join("captures")
        .join(capture.id().as_str())
        .join(INSTANCES_FILE_NAME);
    fs::create_dir(&path).expect("the invalid manifest directory can be created");

    let error = store
        .list_captures()
        .expect_err("a non-file instance manifest must be rejected");

    assert!(matches!(
        error,
        Error::ExpectedInstanceFile { path: error_path } if error_path == path
    ));
}

#[test]
fn rejects_a_completed_directory_without_instances() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    write_completed_record(&store, capture.id().as_str(), &capture);
    let path = store
        .root
        .join("captures")
        .join(capture.id().as_str())
        .join(INSTANCES_FILE_NAME);

    let error = store
        .list_captures()
        .expect_err("a capture without instance evidence must be rejected");

    assert!(matches!(
        error,
        Error::Filesystem {
            operation: "read metadata for",
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
        .list_captures()
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
        .list_captures()
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
        .list_captures()
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
fn rejects_a_manifest_id_that_differs_from_its_directory_id() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let directory_id: CaptureId = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy"
        .parse()
        .expect("the directory capture ID is valid");
    let capture = record(directory_id.as_str(), 1_000);
    write_completed_record(&store, directory_id.as_str(), &capture);
    let manifest_id: CaptureId = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx"
        .parse()
        .expect("the manifest capture ID is valid");
    let instances = manifest(&manifest_id);
    let path = write_completed_manifest(&store, directory_id.as_str(), &instances);

    let error = store
        .read_instances(&directory_id)
        .expect_err("the mismatched manifest ID must be rejected");

    assert!(matches!(
        error,
        Error::MismatchedCaptureId {
            path: error_path,
            directory_id: error_directory_id,
            record_id,
        } if error_path == path
            && error_directory_id == directory_id
            && record_id == manifest_id
    ));
}

#[test]
fn rejects_a_manifest_whose_counts_differ_from_its_capture_header() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record_with_counts("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000, 1, 1);
    write_completed_record(&store, capture.id().as_str(), &capture);
    let instances = manifest(capture.id());
    write_completed_manifest(&store, capture.id().as_str(), &instances);

    let error = store
        .read_instances(capture.id())
        .expect_err("manifest counts that differ from the capture header must be rejected");

    assert!(matches!(
        error,
        Error::MismatchedInstanceCounts {
            capture_id,
            capture_instance_count: 1,
            capture_placement_count: 1,
            manifest_instance_count: 0,
            manifest_placement_count: 0,
        } if capture_id == *capture.id()
    ));
}

#[test]
fn failed_publication_leaves_no_completed_capture() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::new(temporary.path()).expect("the fixture store path is valid");
    let capture = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    let staging = store.root.join("staging").join(capture.id().as_str());
    fs::create_dir_all(&staging).expect("the conflicting staging directory can be created");

    store
        .publish(&capture, &manifest(capture.id()))
        .expect_err("the staging collision must fail publication");

    assert!(
        store
            .list_captures()
            .expect("failed publication is not visible")
            .is_empty()
    );
    assert!(staging.is_dir());
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
