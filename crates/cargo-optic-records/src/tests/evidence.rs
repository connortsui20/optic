//! Checks capture-scoped references and the complete evidence graph.
//!
//! Construction and deserialization share arithmetic, identity, availability, and range checks.

use super::capture_id;
use super::instance;
use super::provenance;
use crate::*;

fn evidence() -> InstanceManifest {
    let source = ArtifactId::new(1);
    let llvm = ArtifactId::new(2);
    let original = instance();
    let captured = InstanceRecord::new(
        original.definition().clone(),
        original.display_name(),
        original.raw_symbol(),
        original.placements().to_vec(),
        SourceAvailability::Available(
            SourceRecord::new(
                source,
                ByteRange::new(2, 8).unwrap(),
                "src/lib.rs".into(),
                1,
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let definitions = vec![
        LlvmDefinitionRecord::new(
            "body",
            ByteRange::new(0, 10).unwrap(),
            LlvmDefinitionKind::Function,
        )
        .unwrap(),
        LlvmDefinitionRecord::new(
            "alias",
            ByteRange::new(10, 10).unwrap(),
            LlvmDefinitionKind::DirectAlias {
                target: "body".into(),
            },
        )
        .unwrap(),
    ];
    InstanceManifest::new(
        capture_id(),
        vec![captured],
        vec![
            ArtifactRecord::new(source, ArtifactKind::Source, 10),
            ArtifactRecord::new(llvm, ArtifactKind::Llvm, 20),
        ],
        provenance(),
        LlvmCollection::Collected(vec![
            LlvmModuleRecord::new(llvm, "cgu.0", LlvmStage::NoLtoOptimized, definitions).unwrap(),
        ]),
    )
    .unwrap()
}

#[test]
fn references_round_trip_without_losing_large_ordinals() {
    for ordinal in [0, 1, u64::MAX] {
        let reference = InstanceRef::new(capture_id(), ordinal);
        assert_eq!(
            reference.to_string().parse::<InstanceRef>().unwrap(),
            reference
        );
        assert_eq!(reference.capture_id(), &capture_id());
        assert_eq!(reference.ordinal(), ordinal);
        assert_eq!(
            serde_json::from_value::<InstanceRef>(serde_json::to_value(&reference).unwrap())
                .unwrap(),
            reference
        );
    }
    for suffix in [
        "",
        "00",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "1:2",
        "18446744073709551616",
    ] {
        assert!(
            format!("{}:{suffix}", capture_id())
                .parse::<InstanceRef>()
                .is_err(),
            "{suffix}"
        );
    }
    assert!("short:0".parse::<InstanceRef>().is_err());
}

#[test]
fn generated_artifact_names_cannot_escape_the_capture() {
    for (id, expected) in [
        (0, "artifact-0000000000000000"),
        (u64::MAX, "artifact-ffffffffffffffff"),
    ] {
        let record = ArtifactRecord::new(ArtifactId::new(id), ArtifactKind::Source, 0);
        assert_eq!(record.file_name(), expected);
        let mut encoded = serde_json::to_value(record).unwrap();
        encoded["file_name"] = "../escape".into();
        assert!(serde_json::from_value::<ArtifactRecord>(encoded).is_err());
    }
}

#[test]
fn byte_ranges_check_end_arithmetic_including_zero_length() {
    for (start, length) in [(0, 0), (u64::MAX, 0), (1_u64 << 32, 100), (u64::MAX - 1, 1)] {
        let range = ByteRange::new(start, length).unwrap();
        assert_eq!(range.end(), start + length);
        assert_eq!(
            serde_json::from_value::<ByteRange>(serde_json::to_value(range).unwrap()).unwrap(),
            range
        );
    }
    assert!(ByteRange::new(u64::MAX, 1).is_err());
    assert!(
        serde_json::from_value::<ByteRange>(serde_json::json!({"start": u64::MAX, "length": 1}))
            .is_err()
    );
}

#[test]
fn complete_evidence_round_trips_with_module_owned_aliases() {
    let manifest = evidence();
    let encoded = serde_json::to_value(&manifest).unwrap();
    assert_eq!(
        serde_json::from_value::<InstanceManifest>(encoded).unwrap(),
        manifest
    );
    assert_eq!(manifest.artifacts().len(), 2);
    assert!(matches!(
        manifest.instances()[0].source(),
        SourceAvailability::Available(_)
    ));
}

#[test]
fn rejects_invalid_source_and_alias_constructors() {
    let range = ByteRange::new(0, 1).unwrap();
    assert!(SourceRecord::new(ArtifactId::new(0), range, "".into(), 1).is_err());
    assert!(SourceRecord::new(ArtifactId::new(0), range, "src/lib.rs".into(), 0).is_err());
    assert!(LlvmDefinitionRecord::new("", range, LlvmDefinitionKind::Function).is_err());
    assert!(
        LlvmDefinitionRecord::new(
            "alias",
            range,
            LlvmDefinitionKind::DirectAlias { target: "".into() }
        )
        .is_err()
    );
}

#[test]
fn rejects_invalid_graphs_and_provenance_on_deserialization() {
    let encoded = serde_json::to_value(evidence()).unwrap();
    for (pointer, value) in [
        ("/artifacts/1/id", serde_json::json!(1)), // Duplicate artifact ID.
        (
            "/instances/0/source/available/artifact",
            serde_json::json!(9),
        ), // Unknown source artifact.
        (
            "/instances/0/source/available/artifact",
            serde_json::json!(2),
        ), // LLVM used as source.
        (
            "/instances/0/source/available/range/length",
            serde_json::json!(9),
        ), // Source beyond EOF.
        (
            "/instances/0/source/available/starting_line",
            serde_json::json!(0),
        ), // Invalid source line.
        ("/llvm/collected/0/artifact", serde_json::json!(1)), // Source used as LLVM.
        (
            "/llvm/collected/0/definitions/0/range/length",
            serde_json::json!(21),
        ), // Definition beyond EOF.
        (
            "/llvm/collected/0/definitions/1/raw_symbol",
            serde_json::json!("body"),
        ), // Duplicate module symbol.
        (
            "/llvm/collected/0/stage",
            serde_json::json!("local_thin_lto_post_pass_manager"),
        ), // Wrong stage.
        ("/llvm_provenance/backend", serde_json::json!("")), // Missing backend.
        (
            "/llvm_provenance/backend",
            serde_json::json!("another-backend"),
        ), // Unsupported backend.
        (
            "/llvm_provenance/lto",
            serde_json::json!("cross_crate_thin"),
        ), // Unsupported cross-crate ThinLTO.
        ("/llvm_provenance/lto", serde_json::json!("fat")), // Unsupported fat LTO.
        (
            "/llvm_provenance/linker_plugin_lto",
            serde_json::json!(true),
        ), // Unsupported linker-plugin LTO.
        ("/llvm_provenance/llvm_version", serde_json::json!(null)), // Missing collected LLVM identity.
        ("/llvm_provenance/optimization", serde_json::json!("fast")), // Unknown optimization.
        ("/llvm_provenance/codegen_units", serde_json::json!(0)),   // Invalid CGUs.
        ("/llvm_provenance/recipe_revision", serde_json::json!(0)), // Invalid recipe.
        ("/llvm_provenance/incremental", serde_json::json!(true)), // Unsupported collected configuration.
        ("/llvm", serde_json::json!({"not_captured": "incremental"})), // Orphan LLVM artifact.
    ] {
        let mut invalid = encoded.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<InstanceManifest>(invalid).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn source_is_required_without_a_legacy_default() {
    let mut encoded = serde_json::to_value(instance()).unwrap();
    encoded.as_object_mut().unwrap().remove("source");
    assert!(serde_json::from_value::<InstanceRecord>(encoded).is_err());
}

#[test]
fn unavailable_reasons_round_trip_and_have_stable_diagnostics() {
    for reason in [
        UnsupportedLlvmConfiguration::Incremental,
        UnsupportedLlvmConfiguration::CrossCrateThinLto,
        UnsupportedLlvmConfiguration::FatLto,
        UnsupportedLlvmConfiguration::LinkerPluginLto,
        UnsupportedLlvmConfiguration::OtherBackend,
        UnsupportedLlvmConfiguration::UnverifiedCompiler,
    ] {
        let manifest = InstanceManifest::new(
            capture_id(),
            vec![instance()],
            vec![],
            provenance(),
            LlvmCollection::NotCaptured(reason),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<InstanceManifest>(serde_json::to_value(&manifest).unwrap())
                .unwrap(),
            manifest
        );
        assert!(!reason.to_string().is_empty());
    }
    assert_eq!(LlvmStage::NoLtoOptimized.to_string(), "optimized, no LTO");
    assert_eq!(
        SourceUnavailable::Nonlocal.to_string(),
        "source is unavailable for a nonlocal definition"
    );
}
