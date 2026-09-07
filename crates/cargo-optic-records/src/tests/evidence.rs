//! Checks capture-scoped references and the complete evidence graph.
//!
//! Construction and deserialization share arithmetic, identity, availability, and range checks.

use super::assert_json_round_trip;
use super::capture_id;
use super::instance_record;
use super::llvm_provenance;
use crate::ArtifactId;
use crate::ArtifactKind;
use crate::ArtifactRecord;
use crate::ByteRange;
use crate::InstanceManifest;
use crate::InstanceRecord;
use crate::InstanceRef;
use crate::LlvmCollection;
use crate::LlvmDefinitionKind;
use crate::LlvmDefinitionRecord;
use crate::LlvmModuleRecord;
use crate::LlvmStage;
use crate::SourceAvailability;
use crate::SourceRecord;
use crate::SourceUnavailable;
use crate::UnsupportedLlvmConfiguration;

/// Builds one instance with source, its LLVM module, and a direct alias owned by that module.
fn collected_manifest() -> InstanceManifest {
    let source = ArtifactId::new(1);
    let llvm = ArtifactId::new(2);
    let original = instance_record();
    let source_record = SourceRecord::new(
        source,
        ByteRange::new(2, 8).unwrap(),
        "src/lib.rs".into(),
        1,
    )
    .unwrap();

    let captured = InstanceRecord::new(
        original.definition().clone(),
        original.display_name(),
        original.raw_symbol(),
        original.placements().to_vec(),
        SourceAvailability::Available(source_record),
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
    let module = LlvmModuleRecord::new(
        llvm,
        "example-cgu.0",
        LlvmStage::NoLtoOptimized,
        definitions,
    )
    .unwrap();

    InstanceManifest::new(
        capture_id(),
        vec![captured],
        vec![
            ArtifactRecord::new(source, ArtifactKind::Source, 10),
            ArtifactRecord::new(llvm, ArtifactKind::Llvm, 20),
        ],
        llvm_provenance(),
        LlvmCollection::Collected(vec![module]),
    )
    .unwrap()
}

#[test]
fn references_round_trip_without_losing_large_ordinals() {
    let capture_id = capture_id();

    for ordinal in [
        0,        // Select the first instance.
        1,        // Select the next instance.
        u64::MAX, // Preserve the largest ordinal.
    ] {
        let reference = InstanceRef::new(capture_id.clone(), ordinal);

        assert_eq!(
            reference.to_string().parse::<InstanceRef>().unwrap(),
            reference
        );
        assert_eq!(reference.capture_id(), &capture_id);
        assert_eq!(reference.ordinal(), ordinal);
        assert_json_round_trip(&reference);
    }
}

#[test]
fn references_reject_noncanonical_ordinals_and_capture_ids() {
    let capture_id = capture_id();

    for suffix in [
        "",                     // Reject a missing ordinal.
        "00",                   // Reject a zero with a leading zero.
        "01",                   // Reject a nonzero ordinal with a leading zero.
        "+1",                   // Reject an explicit positive sign.
        "-1",                   // Reject a negative sign.
        " 1",                   // Reject leading whitespace.
        "1 ",                   // Reject trailing whitespace.
        "1:2",                  // Reject another separator.
        "18446744073709551616", // Reject an ordinal above `u64::MAX`.
    ] {
        assert!(
            format!("{capture_id}:{suffix}")
                .parse::<InstanceRef>()
                .is_err(),
            "{suffix:?}"
        );
    }

    assert!("short:0".parse::<InstanceRef>().is_err());
}

#[test]
fn generated_artifact_names_cannot_escape_the_capture() {
    for (id, expected) in [
        (0, "artifact-0000000000000000"), // Use the minimum artifact ID.
        (u64::MAX, "artifact-ffffffffffffffff"), // Use the maximum artifact ID.
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
    for (start, length) in [
        (0, 0),             // Allow an empty range at the origin.
        (u64::MAX, 0),      // Allow an empty range at the maximum offset.
        (1_u64 << 32, 100), // Preserve an offset beyond 32 bits.
        (u64::MAX - 1, 1),  // End at the maximum offset.
    ] {
        let range = ByteRange::new(start, length).unwrap();

        assert_eq!(range.end(), start + length);
        assert_json_round_trip(&range);
    }

    assert!(ByteRange::new(u64::MAX, 1).is_err());
    assert!(
        serde_json::from_value::<ByteRange>(serde_json::json!({"start": u64::MAX, "length": 1}))
            .is_err()
    );
}

#[test]
fn complete_evidence_round_trips_with_module_owned_aliases() {
    let manifest = collected_manifest();

    assert_json_round_trip(&manifest);
    assert_eq!(manifest.artifacts().len(), 2);
    assert!(matches!(
        manifest.instances()[0].source(),
        SourceAvailability::Available(_)
    ));
}

#[test]
fn collected_llvm_requires_every_placement_module_but_not_its_body() {
    let original = collected_manifest();

    let LlvmCollection::Collected(modules) = original.llvm() else {
        panic!("the evidence fixture contains collected modules");
    };

    let mut artifacts = original.artifacts().to_vec();
    let mut modules = modules.clone();
    let unplaced_artifact = ArtifactId::new(3);
    artifacts.push(ArtifactRecord::new(
        unplaced_artifact,
        ArtifactKind::Llvm,
        0,
    ));
    modules.push(
        LlvmModuleRecord::new(
            unplaced_artifact,
            "unplaced-cgu.0",
            LlvmStage::NoLtoOptimized,
            vec![],
        )
        .unwrap(),
    );

    let complete = InstanceManifest::new(
        capture_id(),
        original.instances().to_vec(),
        artifacts.clone(),
        llvm_provenance(),
        LlvmCollection::Collected(modules.clone()),
    )
    .unwrap();

    assert_json_round_trip(&complete);

    // Removing both entries leaves no orphan artifact, but the placement still requires its CGU.
    let removed = modules.remove(0).artifact();
    artifacts.retain(|artifact| artifact.id() != removed);

    let error = InstanceManifest::new(
        capture_id(),
        original.instances().to_vec(),
        artifacts.clone(),
        llvm_provenance(),
        LlvmCollection::Collected(modules.clone()),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("missing module for placement example-cgu.0")
    );

    let mut incomplete = serde_json::to_value(complete).unwrap();
    incomplete["artifacts"] = serde_json::to_value(artifacts).unwrap();
    incomplete["llvm"] = serde_json::to_value(LlvmCollection::Collected(modules)).unwrap();

    let error = serde_json::from_value::<InstanceManifest>(incomplete).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("missing module for placement example-cgu.0")
    );
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
    let encoded = serde_json::to_value(collected_manifest()).unwrap();

    for (pointer, value) in [
        ("/artifacts/1/id", serde_json::json!(1)), // Reject a duplicate artifact ID.
        (
            "/instances/0/source/available/artifact",
            serde_json::json!(9),
        ), // Require a known source artifact.
        (
            "/instances/0/source/available/artifact",
            serde_json::json!(2),
        ), // Reject LLVM as source.
        (
            "/instances/0/source/available/range/length",
            serde_json::json!(9),
        ), // Reject a range beyond the source.
        (
            "/instances/0/source/available/starting_line",
            serde_json::json!(0),
        ), // Require a nonzero source line.
        ("/llvm/collected/0/artifact", serde_json::json!(1)), // Reject source as LLVM.
        (
            "/llvm/collected/0/definitions/0/range/length",
            serde_json::json!(21),
        ), // Reject a range beyond the LLVM module.
        (
            "/llvm/collected/0/definitions/1/raw_symbol",
            serde_json::json!("body"),
        ), // Reject a duplicate module symbol.
        (
            "/llvm/collected/0/stage",
            serde_json::json!("local_thin_lto_post_pass_manager"),
        ), // Require the recorded LLVM stage.
        ("/llvm_provenance/backend", serde_json::json!("")), // Require the backend identity.
        (
            "/llvm_provenance/backend",
            serde_json::json!("another-backend"),
        ), // Reject an unsupported backend.
        (
            "/llvm_provenance/lto",
            serde_json::json!("cross_crate_thin"),
        ), // Reject cross-crate ThinLTO.
        ("/llvm_provenance/lto", serde_json::json!("fat")), // Reject fat LTO.
        (
            "/llvm_provenance/linker_plugin_lto",
            serde_json::json!(true),
        ), // Reject linker-plugin LTO.
        ("/llvm_provenance/llvm_version", serde_json::json!(null)), // Require the LLVM identity.
        ("/llvm_provenance/optimization", serde_json::json!("fast")), // Reject an unknown level.
        ("/llvm_provenance/codegen_units", serde_json::json!(0)), // Reject zero codegen units.
        ("/llvm_provenance/recipe_revision", serde_json::json!(0)), // Reject a zero revision.
        ("/llvm_provenance/incremental", serde_json::json!(true)), // Reject incremental LLVM.
        ("/llvm", serde_json::json!({"not_captured": "incremental"})), // Reject orphan artifacts.
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
    let mut encoded = serde_json::to_value(instance_record()).unwrap();
    encoded.as_object_mut().unwrap().remove("source");

    assert!(serde_json::from_value::<InstanceRecord>(encoded).is_err());
}

#[test]
fn unavailable_reasons_round_trip_and_render_diagnostics() {
    let capture_id = capture_id();
    let instance = instance_record();
    let provenance = llvm_provenance();

    for reason in [
        UnsupportedLlvmConfiguration::Incremental,
        UnsupportedLlvmConfiguration::CrossCrateThinLto,
        UnsupportedLlvmConfiguration::FatLto,
        UnsupportedLlvmConfiguration::LinkerPluginLto,
        UnsupportedLlvmConfiguration::OtherBackend,
        UnsupportedLlvmConfiguration::UnverifiedCompiler,
    ] {
        let manifest = InstanceManifest::new(
            capture_id.clone(),
            vec![instance.clone()],
            vec![],
            provenance.clone(),
            LlvmCollection::NotCaptured(reason),
        )
        .unwrap();

        assert_json_round_trip(&manifest);
        assert!(!reason.to_string().is_empty());
    }

    assert_eq!(LlvmStage::NoLtoOptimized.to_string(), "optimized, no LTO");
    assert_eq!(
        SourceUnavailable::Nonlocal.to_string(),
        "source is unavailable for a nonlocal definition"
    );
}
