//! Shares fixtures for persistence boundary tests.
//!
//! Sibling suites exercise completed history, candidate reuse, artifact copying, and publication
//! failures.

use std::fs;
use std::path::PathBuf;

use optic_records::BuildRecord;
use optic_records::CaptureAnalysis;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::CargoTargetKind;
use optic_records::CompilerIdentity;
use optic_records::InstanceManifest;
use optic_records::TargetRecord;

use crate::CAPTURE_FILE_NAME;
use crate::INSTANCES_FILE_NAME;
use crate::Store;
use crate::publish::PublicationBoundary;

const PUBLICATION_BOUNDARIES: [PublicationBoundary; 6] = [
    PublicationBoundary::CaptureWrite, // The capture header write fails.
    PublicationBoundary::InstancesWrite, // The instance manifest write fails.
    PublicationBoundary::ArtifactCopy, // The declared artifact copy fails.
    PublicationBoundary::PointerWrite, // The staged pointer write fails.
    PublicationBoundary::PointerReplace, // The pointer installation fails.
    PublicationBoundary::CaptureRename, // The final capture commit fails.
];

mod artifacts;
mod bounds;
mod candidates;
mod captures;
mod initialization;
mod publication;

/// Uses one request key and a different analysis token for each fixture completion time.
fn record(id: &str, completed_at_unix_ms: u64) -> CaptureRecord {
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
        analysis(completed_at_unix_ms),
    )
}

fn analysis(completed_at_unix_ms: u64) -> CaptureAnalysis {
    serde_json::from_value(serde_json::json!({
        "request_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "token": format!("{completed_at_unix_ms:012x}40008000000000000000"),
        "artifact": {
            "package_id": "path+file:///workspace#example@0.1.0",
            "manifest_path": "/workspace/Cargo.toml",
            "target": {
                "name": "example", "kind": ["lib"], "crate_types": ["lib"],
                "src_path": "/workspace/src/lib.rs", "edition": "2024"
            },
            "profile": {
                "opt_level": "3", "debuginfo": 0, "debug_assertions": false,
                "overflow_checks": false, "test": false
            },
            "features": [], "filenames": [], "executable": null, "fresh": false
        }
    }))
    .expect("the analysis fixture is valid")
}

fn manifest(id: &CaptureId) -> InstanceManifest {
    InstanceManifest::new(
        id.clone(),
        Vec::new(),
        Vec::new(),
        provenance(),
        optic_records::LlvmCollection::Collected(vec![]),
    )
    .expect("the fixture instance manifest is valid")
}

fn provenance() -> optic_records::LlvmProvenance {
    optic_records::LlvmProvenance::new(
        "llvm",
        Some("22.1.0".into()),
        "x86_64-unknown-linux-gnu",
        "3",
        optic_records::LlvmLto::Off,
        false,
        false,
        16,
        1,
    )
    .unwrap()
}

#[track_caller]
fn publish_capture(store: &Store, capture: &CaptureRecord) {
    store
        .publish(capture, &manifest(capture.id()), &store.root)
        .expect("the capture can be published");
}

/// Writes a header directly so tests can construct incomplete or mismatched capture directories.
fn write_completed_record(store: &Store, directory_id: &str, capture: &CaptureRecord) -> PathBuf {
    let directory = store.root.join("captures").join(directory_id);
    fs::create_dir_all(&directory).expect("the completed capture directory can be created");
    let path = directory.join(CAPTURE_FILE_NAME);
    let encoded = serde_json::to_vec(capture).expect("the fixture record can be encoded");
    fs::write(&path, encoded).expect("the fixture record can be written");

    path
}

/// Writes a manifest without publication validation so tests can isolate corrupt store layouts.
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
