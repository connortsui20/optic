//! Verifies candidate completeness independently of Cargo freshness.
//!
//! The filesystem cases distinguish explicit misses from present corrupt evidence.

use std::fs;

use super::publish_capture;
use super::record;
use crate::CAPTURE_FILE_NAME;
use crate::INSTANCES_FILE_NAME;
use crate::Store;

#[test]
fn replaces_the_candidate_without_rewriting_history() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
    let key = older.analysis().request_key();
    assert_eq!(store.read_candidate(key).unwrap(), None);

    publish_capture(&store, &older);
    let header_path = store
        .capture_directory(older.id())
        .unwrap()
        .join(CAPTURE_FILE_NAME);
    let original_header = fs::read(&header_path).unwrap();
    assert_eq!(store.read_candidate(key).unwrap(), Some(older.clone()));

    publish_capture(&store, &newer);
    assert_eq!(store.read_candidate(key).unwrap(), Some(newer.clone()));
    assert_eq!(store.read_capture(older.id()).unwrap(), older);
    assert!(
        store
            .read_instances(older.id())
            .unwrap()
            .instances()
            .is_empty()
    );
    assert_eq!(fs::read(header_path).unwrap(), original_header);
    assert_eq!(store.list_captures().unwrap(), vec![newer, older]);
}

#[test]
fn keeps_independent_candidates_for_different_request_keys() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let first = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let second = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
    let mut encoded = serde_json::to_value(second).unwrap();
    encoded["analysis"]["request_key"] = "a".repeat(64).into();
    let second: optic_records::CaptureRecord = serde_json::from_value(encoded).unwrap();
    publish_capture(&store, &first);
    publish_capture(&store, &second);

    assert_eq!(
        store
            .read_candidate(first.analysis().request_key())
            .unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        store
            .read_candidate(second.analysis().request_key())
            .unwrap(),
        Some(second.clone())
    );
    assert_eq!(store.list_captures().unwrap(), vec![second, first]);
}

#[test]
fn absent_and_dangling_pointers_are_misses_without_history_fallback() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let key = capture.analysis().request_key();
    publish_capture(&store, &capture);
    let pointer = store.candidate_path(key);
    let bytes = fs::read(&pointer).unwrap();

    fs::remove_file(&pointer).unwrap();
    assert_eq!(store.read_candidate(key).unwrap(), None);
    assert_eq!(store.read_capture(capture.id()).unwrap(), capture);

    let mut dangling: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    dangling["capture_id"] = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx".into();
    fs::write(&pointer, serde_json::to_vec(&dangling).unwrap()).unwrap();
    assert_eq!(store.read_candidate(key).unwrap(), None);
    assert_eq!(store.read_capture(capture.id()).unwrap(), capture);
    assert_eq!(
        fs::read(&pointer).unwrap(),
        serde_json::to_vec(&dangling).unwrap()
    );
}

#[test]
fn rejects_present_corrupt_pointers_and_captures() {
    let cases = [
        ("pointer", "", serde_json::json!(null)), // Invalid pointer shape.
        ("pointer", "/format_version", serde_json::json!(2)), // Unsupported pointer.
        ("pointer", "/request_key", serde_json::json!("a".repeat(64))), // Wrong request key.
        ("pointer", "/capture_id", serde_json::json!("../escape")), // Invalid capture ID.
        (
            "pointer",
            "/token",
            serde_json::json!("0123456789ab4def8123456789abcdef"),
        ), // Wrong token.
        (
            "capture",
            "/id",
            serde_json::json!("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx"),
        ), // Wrong header ID.
        ("capture", "/format_version", serde_json::json!(2)), // Old capture schema.
        (
            "capture",
            "/analysis/request_key",
            serde_json::json!("a".repeat(64)),
        ), // Wrong header key.
        (
            "capture",
            "/analysis/token",
            serde_json::json!("0123456789ab4def8123456789abcdef"),
        ), // Wrong header token.
        (
            "instances",
            "/capture_id",
            serde_json::json!("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx"),
        ), // Wrong manifest ID.
        ("instances", "/format_version", serde_json::json!(2)), // Old manifest schema.
        ("instances", "/instances", serde_json::json!([{}])), // Incomplete instance.
    ];

    for (file, field, invalid) in cases {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        publish_capture(&store, &capture);
        let directory = store.capture_directory(capture.id()).unwrap();
        let path = match file {
            "pointer" => store.candidate_path(capture.analysis().request_key()),
            "capture" => directory.join(CAPTURE_FILE_NAME),
            "instances" => directory.join(INSTANCES_FILE_NAME),
            _ => unreachable!("the case table names only the three durable files"),
        };
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        *value.pointer_mut(field).unwrap() = invalid;
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();

        assert!(
            store
                .read_candidate(capture.analysis().request_key())
                .is_err(),
            "{file} {field}"
        );
    }
}

#[test]
fn rejects_missing_truncated_and_nonregular_candidate_files() {
    for file in [CAPTURE_FILE_NAME, INSTANCES_FILE_NAME, "pointer"] {
        for corruption in ["missing", "truncated", "directory"] {
            if file == "pointer" && corruption == "missing" {
                continue;
            }

            let temporary = tempfile::tempdir().unwrap();
            let store = Store::new(temporary.path()).unwrap();
            let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
            publish_capture(&store, &capture);
            let path = if file == "pointer" {
                store.candidate_path(capture.analysis().request_key())
            } else {
                store.capture_directory(capture.id()).unwrap().join(file)
            };
            fs::remove_file(&path).unwrap();
            match corruption {
                "missing" => {}
                "truncated" => fs::write(&path, b"{\"format_version\":").unwrap(),
                "directory" => fs::create_dir(&path).unwrap(),
                _ => unreachable!("the case table names only three corruptions"),
            }

            assert!(
                store
                    .read_candidate(capture.analysis().request_key())
                    .is_err(),
                "{file} {corruption}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_at_each_owned_read_boundary() {
    for entry in [
        ".optic",
        "store",
        "captures",
        "candidates",
        "capture-directory",
        CAPTURE_FILE_NAME,
        INSTANCES_FILE_NAME,
        "pointer",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        publish_capture(&store, &capture);
        let directory = store.capture_directory(capture.id()).unwrap();
        let path = match entry {
            ".optic" => temporary.path().join(".optic"),
            "store" => store.root.clone(),
            "captures" | "candidates" => store.root.join(entry),
            "capture-directory" => directory.clone(),
            "pointer" => store.candidate_path(capture.analysis().request_key()),
            _ => directory.join(entry),
        };
        let moved = temporary.path().join("moved");
        fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();

        assert!(
            store
                .read_candidate(capture.analysis().request_key())
                .is_err(),
            "{entry}"
        );
        if entry != "candidates" && entry != "pointer" {
            assert!(store.read_capture(capture.id()).is_err(), "{entry}");
            assert!(store.read_instances(capture.id()).is_err(), "{entry}");
            assert!(store.list_captures().is_err(), "{entry}");
        }

        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(temporary.path().join("absent"), &path).unwrap();
        assert!(
            store
                .read_candidate(capture.analysis().request_key())
                .is_err(),
            "dangling {entry}"
        );
    }
}
