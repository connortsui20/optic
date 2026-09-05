//! Checks query failures at the stored capture boundary.
//!
//! Invalid references and unreadable manifests remain errors even when evidence is unavailable.

use optic_records::CaptureId;
use optic_records::InstanceRef;

use super::TestStore;
use super::instance;
use crate::Error;
use crate::find_instances;
use crate::llvm_evidence;
use crate::source_evidence;

#[test]
fn unavailable_evidence_does_not_hide_invalid_instance_ordinals() {
    let fixture = TestStore::new();
    let id = fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![instance("fixture", "function", "function", "symbol")],
    );

    for ordinal in [1, u64::MAX] {
        let reference = InstanceRef::new(id.clone(), ordinal);

        for error in [
            source_evidence(&fixture.store, &reference).unwrap_err(), //
            llvm_evidence(&fixture.store, &reference).unwrap_err(),   //
        ] {
            assert!(matches!(error, Error::InvalidInstanceReference {
                reference: actual,
                instance_count: 1,
            } if actual == reference));
        }
    }
}

#[test]
fn unknown_captures_remain_store_errors() {
    let fixture = TestStore::new();
    let id: CaptureId = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy".parse().unwrap();
    let reference = InstanceRef::new(id.clone(), 0);

    for error in [
        source_evidence(&fixture.store, &reference).unwrap_err(), //
        llvm_evidence(&fixture.store, &reference).unwrap_err(),   //
    ] {
        assert!(matches!(error, Error::Store {
            source: optic_store::Error::CaptureNotFound { id: actual },
        } if actual == id));
    }
}

#[test]
fn malformed_manifests_preserve_store_error_context() {
    let fixture = TestStore::new();
    let id = fixture.publish("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", Vec::new());
    let path = fixture
        .temporary
        .path()
        .join(".optic/store/captures")
        .join(id.as_str())
        .join("instances.json");
    std::fs::write(&path, b"{}").unwrap();
    let reference = InstanceRef::new(id.clone(), 0);

    for error in [
        find_instances(&fixture.store, &id, "function", 1).unwrap_err(), //
        source_evidence(&fixture.store, &reference).unwrap_err(),        //
        llvm_evidence(&fixture.store, &reference).unwrap_err(),          //
    ] {
        assert!(matches!(error, Error::Store { ref source }
            if source.to_string().contains(&path.display().to_string())));
    }
}
