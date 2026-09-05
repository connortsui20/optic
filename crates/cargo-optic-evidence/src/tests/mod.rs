//! Protects capture-scoped searches and stored evidence selection.
//!
//! Fixtures publish through a real temporary store. Each query must use the captured records and
//! artifacts, including their original instance ordinals.

use std::path::PathBuf;

use optic_records::AnalysisToken;
use optic_records::ArtifactRecord;
use optic_records::BuildRecord;
use optic_records::CaptureAnalysis;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::CargoArtifactRecord;
use optic_records::CargoTargetKind;
use optic_records::CompilerIdentity;
use optic_records::DefinitionRecord;
use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::LlvmCollection;
use optic_records::LlvmLto;
use optic_records::LlvmProvenance;
use optic_records::PlacementRecord;
use optic_records::SourceAvailability;
use optic_records::SourceUnavailable;
use optic_records::TargetRecord;
use optic_records::UnsupportedLlvmConfiguration;
use optic_store::Store;

mod find;

mod references;

mod source;

mod llvm;

struct TestStore {
    /// Keeps the temporary workspace alive for the lifetime of the store handle.
    temporary: tempfile::TempDir,
    /// The store under test.
    store: Store,
}

impl TestStore {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("the test workspace can be created");
        let store = Store::new(temporary.path()).expect("the fixture store path is valid");

        Self { temporary, store }
    }

    fn publish(&self, id: &str, instances: Vec<InstanceRecord>) -> CaptureId {
        self.publish_evidence(
            id,
            instances,
            Vec::new(),
            LlvmCollection::NotCaptured(UnsupportedLlvmConfiguration::Incremental),
        )
    }

    fn publish_evidence(
        &self,
        id: &str,
        instances: Vec<InstanceRecord>,
        artifacts: Vec<(ArtifactRecord, &[u8])>,
        llvm: LlvmCollection,
    ) -> CaptureId {
        let id = id
            .parse::<CaptureId>()
            .expect("the fixture capture ID is valid");
        let target = TargetRecord::new("fixture", CargoTargetKind::Lib)
            .expect("the fixture target is valid");
        let build = BuildRecord::new(
            "fixture",
            "0.1.0",
            target,
            "release",
            PathBuf::from("cargo"),
            self.temporary.path().to_owned(),
            vec!["rustc".to_owned()],
        )
        .expect("the fixture build is valid");
        let sysroot = self.temporary.path().join("toolchain");
        let compiler = CompilerIdentity::new(
            sysroot
                .join("bin")
                .join(format!("rustc{}", std::env::consts::EXE_SUFFIX)),
            "1.99.0-nightly",
            "0123456789abcdef0123456789abcdef01234567",
            "x86_64-unknown-linux-gnu",
            sysroot,
        )
        .expect("the fixture compiler identity is valid");
        let artifact_directory = tempfile::tempdir().expect("the fixture artifacts can be created");

        for (artifact, bytes) in &artifacts {
            std::fs::write(artifact_directory.path().join(artifact.file_name()), bytes)
                .expect("the fixture artifact can be written");
        }

        let manifest = InstanceManifest::new(
            id.clone(),
            instances,
            artifacts
                .into_iter()
                .map(|(artifact, _)| artifact)
                .collect(),
            provenance(&llvm),
            llvm,
        )
        .expect("the fixture instance manifest is valid");
        let artifact = serde_json::from_value(serde_json::json!({
            "package_id": "fixture@0.1.0",
            "manifest_path": self.temporary.path().join("Cargo.toml"),
            "target": {
                "kind": ["lib"],
                "crate_types": ["lib"],
                "name": "fixture",
                "src_path": self.temporary.path().join("src/lib.rs"),
                "edition": "2024",
                "doc": true,
                "doctest": true,
                "test": true,
            },
            "profile": {
                "opt_level": "3",
                "debuginfo": 0,
                "debug_assertions": false,
                "overflow_checks": false,
                "test": false,
            },
            "features": [],
            "filenames": [self.temporary.path().join("target/libfixture.rlib")],
            "executable": null,
            "fresh": false,
        }))
        .expect("the fixture describes a Cargo library artifact");
        let analysis = CaptureAnalysis::new(
            "a".repeat(64)
                .parse()
                .expect("64 hexadecimal digits form a capture key"),
            AnalysisToken::generate(),
            CargoArtifactRecord::new(artifact).expect("the fixture artifact has valid paths"),
        );
        let capture = CaptureRecord::new(id.clone(), 1_000, build, compiler, analysis);

        self.store
            .publish(&capture, &manifest, artifact_directory.path())
            .expect("the fixture capture can be published");

        id
    }
}

fn provenance(llvm: &LlvmCollection) -> LlvmProvenance {
    let reason = match llvm {
        LlvmCollection::NotCaptured(reason) => Some(*reason),
        LlvmCollection::Collected(_) => None,
    };
    let lto = match reason {
        Some(UnsupportedLlvmConfiguration::CrossCrateThinLto) => LlvmLto::CrossCrateThin,
        Some(UnsupportedLlvmConfiguration::FatLto) => LlvmLto::Fat,
        _ => LlvmLto::Off,
    };
    let backend = if reason == Some(UnsupportedLlvmConfiguration::OtherBackend) {
        "other"
    } else {
        "llvm"
    };

    LlvmProvenance::new(
        backend,
        Some("22.1.0".into()),
        "x86_64-unknown-linux-gnu",
        "3",
        lto,
        reason == Some(UnsupportedLlvmConfiguration::Incremental),
        reason == Some(UnsupportedLlvmConfiguration::LinkerPluginLto),
        16,
        1,
    )
    .unwrap()
}

fn instance(
    crate_name: &str,
    definition_path: &str,
    display_name: &str,
    raw_symbol: &str,
) -> InstanceRecord {
    let definition = DefinitionRecord::new(crate_name, definition_path)
        .expect("the fixture definition is valid");
    let placement = PlacementRecord::new("fixture.0", "external", "default", false, 1)
        .expect("the fixture placement is valid");

    InstanceRecord::new(
        definition,
        display_name,
        raw_symbol,
        vec![placement],
        SourceAvailability::Unavailable(SourceUnavailable::Nonlocal),
    )
    .expect("the fixture instance is valid")
}
