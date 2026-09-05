//! Verifies that each precommit failure preserves completed history.
//!
//! Private interruptions cover otherwise inaccessible write and rename boundaries without timing
//! or process-global fault state. Ordinary path obstructions cover filesystem failures directly.

use std::fs;

use super::manifest;
use super::publish_capture;
use super::record;
use crate::CAPTURE_FILE_NAME;
use crate::Store;
use crate::publish::PublicationBoundary;

#[test]
fn each_precommit_failure_keeps_old_evidence_readable() {
    for boundary in [
        PublicationBoundary::CaptureWrite,   // First staged record write.
        PublicationBoundary::InstancesWrite, // Second staged record write.
        PublicationBoundary::PointerWrite,   // Staged pointer write.
        PublicationBoundary::PointerReplace, // Pointer installation.
        PublicationBoundary::CaptureRename,  // Final capture commit.
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let mut store = Store::new(temporary.path()).unwrap();
        let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
        publish_capture(&store, &older);
        let old_header = store
            .capture_directory(older.id())
            .unwrap()
            .join(CAPTURE_FILE_NAME);
        let old_bytes = fs::read(&old_header).unwrap();
        let pointer = store.candidate_path(older.analysis().request_key());
        let old_pointer = fs::read(&pointer).unwrap();
        store.publication_failure = Some(boundary);

        store.publish(&newer, &manifest(newer.id())).unwrap_err();
        assert_eq!(
            store.list_captures().unwrap(),
            vec![older.clone()],
            "{boundary:?}"
        );
        assert_eq!(store.read_capture(older.id()).unwrap(), older);
        assert_eq!(
            store.read_instances(older.id()).unwrap(),
            manifest(older.id())
        );
        assert_eq!(fs::read(old_header).unwrap(), old_bytes);
        assert!(
            !store
                .root
                .join("captures")
                .join(newer.id().as_str())
                .exists()
        );

        if boundary == PublicationBoundary::CaptureRename {
            assert_eq!(
                store
                    .read_candidate(older.analysis().request_key())
                    .unwrap(),
                None
            );
            let pointer: serde_json::Value =
                serde_json::from_slice(&fs::read(pointer).unwrap()).unwrap();
            assert_eq!(pointer["capture_id"], newer.id().as_str());
            assert_eq!(pointer["token"], newer.analysis().token().as_str());
        } else {
            assert_eq!(fs::read(pointer).unwrap(), old_pointer);
            assert_eq!(
                store
                    .read_candidate(older.analysis().request_key())
                    .unwrap(),
                Some(older.clone())
            );
        }

        store.publication_failure = None;
        let next = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzw", 3_000);
        publish_capture(&store, &next);
        assert_eq!(
            store.read_candidate(next.analysis().request_key()).unwrap(),
            Some(next.clone())
        );
        assert_eq!(store.list_captures().unwrap(), vec![next, older]);
    }
}

#[test]
fn obstructed_staging_preserves_the_old_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
    publish_capture(&store, &older);
    let staging = store.root.join("staging").join(newer.id().as_str());
    fs::write(&staging, b"obstruction").unwrap();

    store.publish(&newer, &manifest(newer.id())).unwrap_err();
    assert_eq!(
        store
            .read_candidate(older.analysis().request_key())
            .unwrap(),
        Some(older.clone())
    );
    assert_eq!(store.list_captures().unwrap(), vec![older.clone()]);
    assert_eq!(
        store.read_instances(older.id()).unwrap(),
        manifest(older.id())
    );
    assert_eq!(fs::read(staging).unwrap(), b"obstruction");
}

#[test]
fn obstructed_pointer_replacement_does_not_publish_a_capture() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
    publish_capture(&store, &older);
    let pointer = store.candidate_path(older.analysis().request_key());
    fs::remove_file(&pointer).unwrap();
    fs::create_dir(&pointer).unwrap();

    store.publish(&newer, &manifest(newer.id())).unwrap_err();
    assert_eq!(store.list_captures().unwrap(), vec![older.clone()]);
    assert_eq!(store.read_capture(older.id()).unwrap(), older);
    assert_eq!(
        store.read_instances(older.id()).unwrap(),
        manifest(older.id())
    );
    assert!(pointer.is_dir());
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_publication_namespaces() {
    for namespace in ["staging", "captures", "candidates"] {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        store.initialize().unwrap();
        let path = store.root.join(namespace);
        fs::remove_dir(&path).unwrap();
        let outside = temporary.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);

        store
            .publish(&capture, &manifest(capture.id()))
            .unwrap_err();
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}
