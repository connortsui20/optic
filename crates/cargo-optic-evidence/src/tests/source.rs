//! Checks source selection and explicit compiler unavailability.
//!
//! Source paths remain display metadata. The returned ranges identify only the captured snapshots.

use optic_records::ArtifactId;
use optic_records::ArtifactKind;
use optic_records::ArtifactRecord;
use optic_records::ByteRange;
use optic_records::InstanceRecord;
use optic_records::InstanceRef;
use optic_records::LlvmCollection;
use optic_records::SourceAvailability;
use optic_records::SourceRecord;
use optic_records::SourceUnavailable;

use super::TestStore;
use super::instance;
use crate::SourceEvidence;
use crate::source_evidence;

fn source_instance(source: SourceAvailability) -> InstanceRecord {
    let instance = instance(
        "fixture",
        "fixture::function",
        "fixture::function",
        "symbol",
    );

    InstanceRecord::new(
        instance.definition().clone(),
        instance.display_name(),
        instance.raw_symbol(),
        instance.placements().to_vec(),
        source,
    )
    .unwrap()
}

#[test]
fn available_source_preserves_snapshot_range_and_display_metadata() {
    let fixture = TestStore::new();
    let prefix = "// λ\n";
    let excerpt = "fn function() {}\n";
    let snapshot = format!("{prefix}{excerpt}// retained suffix\n");
    let artifact = ArtifactRecord::new(
        ArtifactId::new(7),
        ArtifactKind::Source,
        snapshot.len() as u64,
    );
    let range = ByteRange::new(prefix.len() as u64, excerpt.len() as u64).unwrap();
    let display_path = fixture.temporary.path().join("missing directory/source.rs");
    let source = SourceRecord::new(artifact.id(), range, display_path.clone(), 2).unwrap();
    let id = fixture.publish_evidence(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![source_instance(SourceAvailability::Available(source))],
        vec![(artifact, snapshot.as_bytes())],
        LlvmCollection::Collected(Vec::new()),
    );

    let SourceEvidence::Available {
        evidence,
        display_path: actual_path,
        starting_line,
    } = source_evidence(&fixture.store, &InstanceRef::new(id.clone(), 0)).unwrap()
    else {
        panic!("the fixture supplies available source");
    };
    let mut bytes = Vec::new();
    fixture
        .store
        .copy_evidence(
            evidence.capture_id(),
            evidence.artifact(),
            evidence.range(),
            &mut bytes,
        )
        .unwrap();

    assert_eq!(evidence.capture_id(), &id);
    assert_eq!(evidence.artifact(), ArtifactId::new(7));
    assert_eq!(evidence.range(), range);
    assert_eq!(actual_path, display_path);
    assert_eq!(starting_line, 2);
    assert_eq!(bytes, excerpt.as_bytes());
    assert!(!display_path.exists());
}

#[test]
fn unavailable_source_preserves_each_recorded_reason() {
    for reason in [
        SourceUnavailable::Nonlocal,        //
        SourceUnavailable::Unloaded,        //
        SourceUnavailable::Generated,       //
        SourceUnavailable::UnsupportedSpan, //
        SourceUnavailable::OutsidePackage,  //
    ] {
        let fixture = TestStore::new();
        let id = fixture.publish(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
            vec![source_instance(SourceAvailability::Unavailable(reason))],
        );

        assert_eq!(
            source_evidence(&fixture.store, &InstanceRef::new(id, 0)).unwrap(),
            SourceEvidence::Unavailable(reason)
        );
    }
}

#[test]
fn a_missing_snapshot_is_a_store_error_with_artifact_context() {
    let fixture = TestStore::new();
    let artifact = ArtifactRecord::new(ArtifactId::new(7), ArtifactKind::Source, 4);
    let source = SourceRecord::new(
        artifact.id(),
        ByteRange::new(0, 4).unwrap(),
        "source.rs".into(),
        1,
    )
    .unwrap();
    let id = fixture.publish_evidence(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![source_instance(SourceAvailability::Available(source))],
        vec![(artifact.clone(), b"body")],
        LlvmCollection::Collected(Vec::new()),
    );
    let path = fixture
        .temporary
        .path()
        .join(".optic/store/captures")
        .join(id.as_str())
        .join(artifact.file_name());
    std::fs::remove_file(&path).unwrap();

    let error = source_evidence(&fixture.store, &InstanceRef::new(id, 0)).unwrap_err();

    assert!(
        matches!(error, crate::Error::Store { ref source } if source.to_string().contains(&path.display().to_string()))
    );
}
