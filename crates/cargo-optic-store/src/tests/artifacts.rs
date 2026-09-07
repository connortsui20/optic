//! Exercises artifact publication and finite range copying with real files.
//!
//! Corruption remains distinct from source unavailability and caller-writer failures.

use std::fs;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;

use optic_records::ArtifactId;
use optic_records::ArtifactKind;
use optic_records::ArtifactRecord;
use optic_records::ByteRange;
use optic_records::CaptureId;
use optic_records::DefinitionRecord;
use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::LlvmCollection;
use optic_records::LlvmModuleRecord;
use optic_records::LlvmStage;
use optic_records::PlacementRecord;
use optic_records::SourceAvailability;
use optic_records::SourceUnavailable;
use optic_records::UnsupportedLlvmConfiguration;

use super::PUBLICATION_BOUNDARIES;
use super::manifest;
use super::provenance;
use super::publish_capture;
use super::record;
use super::write_completed_manifest;
use super::write_completed_record;
use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::Store;
use crate::artifacts::copy_evidence_bytes;
use crate::publish::PublicationBoundary;

/// Declares one source artifact with ID 7 and the selected byte length.
fn artifact_manifest(id: &CaptureId, length: u64) -> InstanceManifest {
    InstanceManifest::new(
        id.clone(),
        vec![],
        vec![ArtifactRecord::new(
            ArtifactId::new(7),
            ArtifactKind::Source,
            length,
        )],
        provenance(),
        LlvmCollection::NotCaptured(UnsupportedLlvmConfiguration::UnverifiedCompiler),
    )
    .unwrap()
}

/// The file layouts that fail validation against a declared three-byte artifact.
#[derive(Clone, Copy, Debug)]
enum ArtifactCorruption {
    Missing,
    Short,
    Long,
    Directory,
}

impl ArtifactCorruption {
    const CASES: [Self; 4] = [
        Self::Missing,   // The artifact is absent.
        Self::Short,     // The artifact has two bytes instead of three.
        Self::Long,      // The artifact has four bytes instead of three.
        Self::Directory, // The artifact is a directory.
    ];

    /// Creates the corrupt fixture at a path that **must** be absent.
    #[track_caller]
    fn write(self, path: &Path) {
        match self {
            Self::Missing => {}
            Self::Short => fs::write(path, b"ab").unwrap(),
            Self::Long => fs::write(path, b"abcd").unwrap(),
            Self::Directory => fs::create_dir(path).unwrap(),
        }
    }
}

/// A caller-owned writer that accepts a prefix and then reports its chosen I/O error.
struct FailingWriter {
    accepted: Vec<u8>,
    limit: usize,
    kind: io::ErrorKind,
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.accepted.len() == self.limit {
            return Err(io::Error::new(self.kind, "caller writer failure"));
        }

        let count = bytes.len().min(self.limit - self.accepted.len());
        self.accepted.extend_from_slice(&bytes[..count]);

        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn publishes_only_declared_artifacts_and_reads_stored_ranges() {
    let temporary = tempfile::tempdir().unwrap();
    let inputs = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let bytes = "αβ\nfn kernel() {}\n".as_bytes();
    let manifest = artifact_manifest(capture.id(), bytes.len() as u64);
    let artifact = &manifest.artifacts()[0];
    let input = inputs.path().join(artifact.file_name());
    fs::write(&input, bytes).unwrap();
    fs::write(inputs.path().join("undeclared.txt"), b"not evidence").unwrap();

    store.publish(&capture, &manifest, inputs.path()).unwrap();
    fs::write(&input, b"changed checkout and temporary bytes").unwrap();
    let mut copied = Vec::new();
    store
        .copy_evidence(
            capture.id(),
            artifact.id(),
            ByteRange::new(5, 15).unwrap(),
            &mut copied,
        )
        .unwrap();

    assert_eq!(copied, b"fn kernel() {}\n");
    assert_eq!(
        store
            .read_candidate(capture.analysis().request_key())
            .unwrap(),
        Some((capture.clone(), manifest.clone()))
    );
    assert_eq!(store.read_instances(capture.id()).unwrap(), manifest);
    assert!(
        !store
            .capture_directory(capture.id())
            .unwrap()
            .join("undeclared.txt")
            .exists()
    );
    assert_eq!(
        fs::read(
            store
                .capture_directory(capture.id())
                .unwrap()
                .join(artifact.file_name())
        )
        .unwrap(),
        bytes
    );
}

#[test]
fn missing_placement_modules_are_corruption_even_without_orphan_artifacts() {
    let temporary = tempfile::tempdir().unwrap();
    let inputs = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let artifact = ArtifactRecord::new(ArtifactId::new(0), ArtifactKind::Llvm, 0);
    let instance = InstanceRecord::new(
        DefinitionRecord::new("example", "example::kernel").unwrap(),
        "example::kernel",
        "kernel",
        vec![PlacementRecord::new("cgu.0", "External", "Default", false, 1).unwrap()],
        SourceAvailability::Unavailable(SourceUnavailable::Nonlocal),
    )
    .unwrap();
    let evidence = InstanceManifest::new(
        capture.id().clone(),
        vec![instance],
        vec![artifact.clone()],
        provenance(),
        LlvmCollection::Collected(vec![
            LlvmModuleRecord::new(artifact.id(), "cgu.0", LlvmStage::NoLtoOptimized, vec![])
                .unwrap(),
        ]),
    )
    .unwrap();
    fs::write(inputs.path().join(artifact.file_name()), []).unwrap();
    store.publish(&capture, &evidence, inputs.path()).unwrap();
    assert_eq!(
        store
            .read_candidate(capture.analysis().request_key())
            .unwrap(),
        Some((capture.clone(), evidence.clone()))
    );

    let mut corrupt = serde_json::to_value(evidence).unwrap();
    corrupt["artifacts"] = serde_json::json!([]);
    corrupt["llvm"]["collected"] = serde_json::json!([]);
    let path = store
        .capture_directory(capture.id())
        .unwrap()
        .join(INSTANCES_FILE_NAME);
    fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();

    for error in [
        store.read_instances(capture.id()).unwrap_err(),
        store
            .read_candidate(capture.analysis().request_key())
            .unwrap_err(),
    ] {
        let Error::Json {
            path: actual,
            source,
        } = error
        else {
            panic!("the invalid manifest must fail deserialization: {error}");
        };
        assert_eq!(actual, path);
        assert!(
            source
                .to_string()
                .contains("missing module for placement cgu.0")
        );
    }
}

#[test]
fn invalid_inputs_leave_previous_capture_and_candidate_intact() {
    for corruption in ArtifactCorruption::CASES {
        let temporary = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
        publish_capture(&store, &older);
        let evidence = artifact_manifest(newer.id(), 3);
        let path = inputs.path().join(evidence.artifacts()[0].file_name());
        corruption.write(&path);

        store.publish(&newer, &evidence, inputs.path()).unwrap_err();
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
}

#[test]
fn corrupt_artifacts_are_errors_in_both_manifest_and_candidate_reads() {
    for corruption in ArtifactCorruption::CASES {
        let temporary = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        let evidence = artifact_manifest(capture.id(), 3);
        let name = evidence.artifacts()[0].file_name();
        fs::write(inputs.path().join(&name), b"abc").unwrap();
        store.publish(&capture, &evidence, inputs.path()).unwrap();
        let path = store.capture_directory(capture.id()).unwrap().join(name);
        fs::remove_file(&path).unwrap();
        corruption.write(&path);

        assert!(
            store
                .read_candidate(capture.analysis().request_key())
                .is_err(),
            "{corruption:?}"
        );
        assert!(
            store.read_instances(capture.id()).is_err(),
            "{corruption:?}"
        );
        let mut output = Vec::new();
        assert!(
            store
                .copy_evidence(
                    capture.id(),
                    ArtifactId::new(7),
                    ByteRange::new(0, 1).unwrap(),
                    &mut output
                )
                .is_err()
        );
        assert!(output.is_empty());
    }
}

#[test]
fn validates_every_artifact_even_when_copying_another_one() {
    let temporary = tempfile::tempdir().unwrap();
    let inputs = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let evidence = InstanceManifest::new(
        capture.id().clone(),
        vec![],
        vec![
            ArtifactRecord::new(ArtifactId::new(7), ArtifactKind::Source, 3),
            ArtifactRecord::new(ArtifactId::new(8), ArtifactKind::Source, 3),
        ],
        provenance(),
        LlvmCollection::Collected(vec![]),
    )
    .unwrap();

    for artifact in evidence.artifacts() {
        fs::write(inputs.path().join(artifact.file_name()), b"abc").unwrap();
    }

    store.publish(&capture, &evidence, inputs.path()).unwrap();
    fs::remove_file(
        store
            .capture_directory(capture.id())
            .unwrap()
            .join(evidence.artifacts()[1].file_name()),
    )
    .unwrap();

    assert!(
        store
            .read_candidate(capture.analysis().request_key())
            .is_err()
    );
    assert!(
        store
            .copy_evidence(
                capture.id(),
                ArtifactId::new(7),
                ByteRange::new(0, 1).unwrap(),
                &mut Vec::new()
            )
            .is_err()
    );
}

#[test]
fn range_reads_keep_offsets_above_four_gibibytes() {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let start = (1_u64 << 32) + 17;
    let evidence = artifact_manifest(capture.id(), start + 3);
    write_completed_record(&store, capture.id().as_str(), &capture);
    write_completed_manifest(&store, capture.id().as_str(), &evidence);
    let path = store
        .capture_directory(capture.id())
        .unwrap()
        .join(evidence.artifacts()[0].file_name());
    let mut sparse = fs::File::create(path).unwrap();
    sparse.set_len(start + 3).unwrap();
    sparse.seek(SeekFrom::Start(start)).unwrap();
    sparse.write_all(b"end").unwrap();
    let mut output = Vec::new();

    store
        .copy_evidence(
            capture.id(),
            ArtifactId::new(7),
            ByteRange::new(start, 3).unwrap(),
            &mut output,
        )
        .unwrap();
    assert_eq!(output, b"end");
}

#[test]
fn validates_unknown_ids_bounds_and_empty_ranges() {
    let temporary = tempfile::tempdir().unwrap();
    let inputs = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let evidence = artifact_manifest(capture.id(), 3);
    fs::write(
        inputs.path().join(evidence.artifacts()[0].file_name()),
        b"abc",
    )
    .unwrap();
    store.publish(&capture, &evidence, inputs.path()).unwrap();

    assert!(matches!(
        store.copy_evidence(
            capture.id(),
            ArtifactId::new(9),
            ByteRange::new(0, 0).unwrap(),
            &mut Vec::new()
        ),
        Err(Error::UnknownArtifact { .. })
    ));
    for range in [ByteRange::new(2, 2).unwrap(), ByteRange::new(4, 0).unwrap()] {
        assert!(matches!(
            store.copy_evidence(capture.id(), ArtifactId::new(7), range, &mut Vec::new()),
            Err(Error::ArtifactRange { .. })
        ));
    }
    let mut writer = FailingWriter {
        accepted: vec![],
        limit: 0,
        kind: io::ErrorKind::BrokenPipe,
    };
    store
        .copy_evidence(
            capture.id(),
            ArtifactId::new(7),
            ByteRange::new(3, 0).unwrap(),
            &mut writer,
        )
        .unwrap();
    assert!(writer.accepted.is_empty());
}

#[test]
fn empty_artifacts_publish_and_copy_successfully() {
    let temporary = tempfile::tempdir().unwrap();
    let inputs = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let evidence = artifact_manifest(capture.id(), 0);
    fs::write(inputs.path().join(evidence.artifacts()[0].file_name()), []).unwrap();

    store.publish(&capture, &evidence, inputs.path()).unwrap();
    store
        .copy_evidence(
            capture.id(),
            ArtifactId::new(7),
            ByteRange::new(0, 0).unwrap(),
            &mut Vec::new(),
        )
        .unwrap();
}

#[test]
fn publication_failures_preserve_old_artifacts_and_invalidate_only_after_pointer_replacement() {
    for boundary in PUBLICATION_BOUNDARIES {
        let temporary = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let mut store = Store::new(temporary.path()).unwrap();
        let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);
        let old_manifest = artifact_manifest(older.id(), 3);
        let new_manifest = artifact_manifest(newer.id(), 3);
        let name = old_manifest.artifacts()[0].file_name();
        fs::write(inputs.path().join(&name), b"old").unwrap();
        store.publish(&older, &old_manifest, inputs.path()).unwrap();
        fs::write(inputs.path().join(&name), b"new").unwrap();
        store.publication_failure = Some(boundary);

        store
            .publish(&newer, &new_manifest, inputs.path())
            .unwrap_err();
        let mut output = Vec::new();
        store
            .copy_evidence(
                older.id(),
                ArtifactId::new(7),
                ByteRange::new(0, 3).unwrap(),
                &mut output,
            )
            .unwrap();
        assert_eq!(output, b"old", "{boundary:?}");
        assert_eq!(store.list_captures().unwrap(), vec![older.clone()]);
        let expected = if boundary == PublicationBoundary::CaptureRename {
            None
        } else {
            Some((older.clone(), old_manifest))
        };
        assert_eq!(
            store
                .read_candidate(older.analysis().request_key())
                .unwrap(),
            expected,
            "{boundary:?}"
        );
    }
}

#[test]
fn distinguishes_caller_writer_failures_from_artifact_read_failures() {
    let temporary = tempfile::tempdir().unwrap();
    let inputs = tempfile::tempdir().unwrap();
    let store = Store::new(temporary.path()).unwrap();
    let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let evidence = artifact_manifest(capture.id(), 3);
    fs::write(
        inputs.path().join(evidence.artifacts()[0].file_name()),
        b"abc",
    )
    .unwrap();
    store.publish(&capture, &evidence, inputs.path()).unwrap();
    for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::Other] {
        let mut writer = FailingWriter {
            accepted: vec![],
            limit: 1,
            kind,
        };
        let error = store
            .copy_evidence(
                capture.id(),
                ArtifactId::new(7),
                ByteRange::new(0, 3).unwrap(),
                &mut writer,
            )
            .unwrap_err();
        assert!(matches!(error, Error::WriteEvidence { source } if source.kind() == kind));
        assert_eq!(writer.accepted, b"a");
    }
}

#[test]
fn detects_premature_eof_after_metadata_and_keeps_reader_error_context() {
    let input = vec![b'x'; 70_000];
    let mut output = Vec::new();
    let error = copy_evidence_bytes(
        &mut input.as_slice(),
        80_000,
        &mut output,
        Path::new("stored-artifact"),
    )
    .unwrap_err();
    let Error::Filesystem { source, .. } = error else {
        panic!("the truncated reader must return a filesystem error, got {error}");
    };

    assert_eq!(source.kind(), io::ErrorKind::UnexpectedEof);
    assert!(!output.is_empty());
    assert!(output.len() < input.len());

    struct BrokenReader;
    impl Read for BrokenReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }
    let error = copy_evidence_bytes(
        &mut BrokenReader,
        1,
        &mut Vec::new(),
        Path::new("stored-artifact"),
    )
    .unwrap_err();
    let Error::Filesystem { source, .. } = error else {
        panic!("the broken reader must return a filesystem error, got {error}");
    };

    assert_eq!(source.kind(), io::ErrorKind::BrokenPipe);
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_input_directories_and_input_or_stored_artifacts() {
    for boundary in ["input-directory", "input-file", "stored-file"] {
        let temporary = tempfile::tempdir().unwrap();
        let inputs = tempfile::tempdir().unwrap();
        let store = Store::new(temporary.path()).unwrap();
        let capture = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
        let evidence = artifact_manifest(capture.id(), 3);
        let name = evidence.artifacts()[0].file_name();
        let input = inputs.path().join(&name);
        fs::write(&input, b"abc").unwrap();
        let outside = temporary.path().join("outside");
        fs::write(&outside, b"abc").unwrap();

        match boundary {
            "input-directory" => {
                let link = temporary.path().join("input-link");
                std::os::unix::fs::symlink(inputs.path(), &link).unwrap();
                assert!(matches!(
                    store.publish(&capture, &evidence, &link),
                    Err(Error::UnexpectedFileType { .. })
                ));
            }
            "input-file" => {
                fs::remove_file(&input).unwrap();
                std::os::unix::fs::symlink(&outside, &input).unwrap();
                assert!(matches!(
                    store.publish(&capture, &evidence, inputs.path()),
                    Err(Error::UnexpectedFileType { .. })
                ));
            }
            "stored-file" => {
                store.publish(&capture, &evidence, inputs.path()).unwrap();
                let stored = store.capture_directory(capture.id()).unwrap().join(&name);
                fs::remove_file(&stored).unwrap();
                std::os::unix::fs::symlink(&outside, &stored).unwrap();
                assert!(
                    store
                        .read_candidate(capture.analysis().request_key())
                        .is_err()
                );
                assert!(
                    store
                        .copy_evidence(
                            capture.id(),
                            ArtifactId::new(7),
                            ByteRange::new(0, 3).unwrap(),
                            &mut Vec::new()
                        )
                        .is_err()
                );
            }
            _ => unreachable!("the case table names only three boundaries"),
        }

        assert_eq!(fs::read(outside).unwrap(), b"abc");
    }
}
