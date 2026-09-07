//! Verifies encoded-size limits before deserialization and publication.
//!
//! Sparse files exercise metadata limits. A bounded reader exercises growth after metadata, and
//! actual oversized records prove that publication applies the reader's limits to writer output.

use std::fs;
use std::io::Cursor;

use optic_records::CaptureRecord;
use optic_records::DefinitionRecord;
use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::PlacementRecord;

use super::manifest;
use super::publish_capture;
use super::record;
use crate::CAPTURE_FILE_NAME;
use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::MAX_HEADER_BYTES;
use crate::MAX_INSTANCES_BYTES;
use crate::Store;
use crate::record_io::read_bounded_record;
use crate::record_io::read_record;
use crate::record_io::write_record;

#[test]
fn rejects_oversized_durable_files_before_json_decoding() {
    for (file, limit) in [
        (CAPTURE_FILE_NAME, MAX_HEADER_BYTES),      // Capture header.
        (INSTANCES_FILE_NAME, MAX_INSTANCES_BYTES), // Instance manifest.
        ("pointer", MAX_HEADER_BYTES),              // Candidate pointer.
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        publish_capture(&store, &capture);
        let path = if file == "pointer" {
            store.candidate_path(capture.analysis().request_key())
        } else {
            store.capture_directory(capture.id()).unwrap().join(file)
        };
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(limit + 1)
            .unwrap();

        let error = store
            .read_candidate(capture.analysis().request_key())
            .unwrap_err();
        let Error::RecordTooLarge {
            limit: actual_limit,
            actual,
            ..
        } = error
        else {
            panic!("the oversized fixture must return a size error, got {error}");
        };

        assert_eq!(actual_limit, limit);
        assert_eq!(actual, limit + 1);
        if file != "pointer" {
            assert!(matches!(
                store.read_instances(capture.id()),
                Err(Error::RecordTooLarge { .. })
            ));
        }
    }
}

#[test]
fn bounds_the_actual_read_before_deserialization() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("growing.json");
    let mut input = Cursor::new(b"not JSON and larger than the checked metadata length");
    let error = read_bounded_record::<serde_json::Value>(&mut input, &path, 8).unwrap_err();

    assert!(matches!(
        error,
        Error::RecordTooLarge {
            limit: 8,
            actual: 9,
            ..
        }
    ));
    assert_eq!(input.position(), 9);
}

#[test]
fn counts_encoded_bytes_and_the_final_newline_at_the_writer_boundary() {
    let temporary = tempfile::tempdir().unwrap();
    let value = "é\n\"";
    let encoded = serde_json::to_vec(value).unwrap();
    let exact_limit = encoded.len() as u64 + 1;
    let path = temporary.path().join("exact.json");
    write_record(&path, &value, exact_limit).unwrap();

    assert_eq!(fs::metadata(&path).unwrap().len(), exact_limit);
    assert_eq!(read_record::<String>(&path, exact_limit).unwrap(), value);

    for limit in [exact_limit - 1, exact_limit - 2] {
        let path = temporary.path().join(format!("over-{limit}.json"));
        assert!(matches!(
            write_record(&path, &value, limit),
            Err(Error::RecordTooLarge { .. })
        ));
        assert!(fs::metadata(path).unwrap().len() <= limit);
    }
}

#[test]
fn oversized_capture_output_preserves_the_old_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
    publish_capture(&store, &older);
    let mut oversized = serde_json::to_value(&newer).unwrap();
    oversized["build"]["cargo_arguments"] =
        serde_json::json!(["x".repeat(MAX_HEADER_BYTES as usize)]);
    let oversized: CaptureRecord = serde_json::from_value(oversized).unwrap();

    assert!(matches!(
        store.publish(&oversized, &manifest(newer.id()), &store.root),
        Err(Error::RecordTooLarge {
            limit: MAX_HEADER_BYTES,
            ..
        })
    ));
    assert_eq!(
        store
            .read_candidate(older.analysis().request_key())
            .unwrap(),
        Some((older.clone(), manifest(older.id())))
    );
    assert_eq!(store.list_captures().unwrap(), vec![older.clone()]);
    assert_eq!(store.read_capture(older.id()).unwrap(), older);
}

#[test]
fn oversized_instance_output_preserves_the_old_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
    publish_capture(&store, &older);
    let instance = InstanceRecord::new(
        DefinitionRecord::new("example", "example::kernel").unwrap(),
        "x".repeat(MAX_INSTANCES_BYTES as usize),
        "_Rkernel",
        vec![PlacementRecord::new("cgu.0", "External", "Default", false, 1).unwrap()],
        optic_records::SourceAvailability::Unavailable(optic_records::SourceUnavailable::Nonlocal),
    )
    .unwrap();
    let oversized = InstanceManifest::new(
        newer.id().clone(),
        vec![instance],
        vec![],
        super::provenance(),
        optic_records::LlvmCollection::NotCaptured(
            optic_records::UnsupportedLlvmConfiguration::UnverifiedCompiler,
        ),
    )
    .unwrap();

    assert!(matches!(
        store.publish(&newer, &oversized, &store.root),
        Err(Error::RecordTooLarge {
            limit: MAX_INSTANCES_BYTES,
            ..
        })
    ));
    assert_eq!(
        store
            .read_candidate(older.analysis().request_key())
            .unwrap(),
        Some((older.clone(), manifest(older.id())))
    );
    assert_eq!(store.list_captures().unwrap(), vec![older.clone()]);
    assert_eq!(
        store.read_instances(older.id()).unwrap(),
        manifest(older.id())
    );
}
