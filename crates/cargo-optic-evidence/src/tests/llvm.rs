//! Checks exact symbol resolution across complete stored module indexes.
//!
//! Artifact bytes are deliberately not LLVM syntax. These queries must trust validated indexes and
//! leave parsing and byte copying to their owning boundaries.

use optic_records::ArtifactId;
use optic_records::ArtifactKind;
use optic_records::ArtifactRecord;
use optic_records::ByteRange;
use optic_records::InstanceRef;
use optic_records::LlvmCollection;
use optic_records::LlvmDefinitionKind;
use optic_records::LlvmDefinitionRecord;
use optic_records::LlvmModuleRecord;
use optic_records::LlvmStage;
use optic_records::UnsupportedLlvmConfiguration;

use super::TestStore;
use super::instance;
use crate::Error;
use crate::LlvmEvidence;
use crate::llvm_evidence;

/// Publishes modules with fixed 32-byte artifacts and an extra `fixture.0` placement module.
///
/// Definition ranges **must** fit those artifacts. Supplied module names **must** exclude
/// `fixture.0`, which this helper reserves for the instance's original compiler placement.
fn publish_modules(
    fixture: &TestStore,
    raw_symbol: &str,
    mut modules: Vec<LlvmModuleRecord>,
) -> InstanceRef {
    // The recorded placement has no retained body. Other modules exercise cross-module lookup.
    let placement_artifact = modules
        .iter()
        .map(|module| module.artifact().value())
        .max()
        .unwrap_or(0)
        + 1;
    modules.push(module(placement_artifact, "fixture.0", Vec::new()));

    let bytes = b"0123456789abcdef0123456789abcdef";
    let artifacts = modules
        .iter()
        .map(|module| {
            (
                ArtifactRecord::new(module.artifact(), ArtifactKind::Llvm, bytes.len() as u64),
                bytes.as_slice(),
            )
        })
        .collect();
    let id = fixture.publish_evidence(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![instance("fixture", "display", "display", raw_symbol)],
        artifacts,
        LlvmCollection::Collected(modules),
    );

    InstanceRef::new(id, 0)
}

fn module(id: u64, name: &str, definitions: Vec<LlvmDefinitionRecord>) -> LlvmModuleRecord {
    LlvmModuleRecord::new(
        ArtifactId::new(id),
        name,
        LlvmStage::NoLtoOptimized,
        definitions,
    )
    .unwrap()
}

fn definition(symbol: &str, start: u64, kind: LlvmDefinitionKind) -> LlvmDefinitionRecord {
    LlvmDefinitionRecord::new(symbol, ByteRange::new(start, 4).unwrap(), kind).unwrap()
}

fn alias(symbol: &str, target: &str) -> LlvmDefinitionRecord {
    definition(
        symbol,
        0,
        LlvmDefinitionKind::DirectAlias {
            target: target.into(),
        },
    )
}

#[test]
fn exact_decoded_quoted_symbols_match_without_display_or_substring_joins() {
    let fixture = TestStore::new();
    let symbol = "quoted \"name\"\\suffix";
    let reference = publish_modules(
        &fixture,
        symbol,
        vec![module(
            5,
            "unplaced.cgu",
            vec![
                definition("display", 0, LlvmDefinitionKind::Function), //
                definition(&format!("{symbol}.extra"), 4, LlvmDefinitionKind::Function), //
                definition(symbol, 8, LlvmDefinitionKind::Function),    //
            ],
        )],
    );

    let LlvmEvidence::Available(bodies) = llvm_evidence(&fixture.store, &reference).unwrap() else {
        panic!("the exact decoded symbol has a function definition");
    };

    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0].raw_symbol(), symbol);
    assert!(bodies[0].aliases().is_empty());
    assert_eq!(bodies[0].compiler_module(), "unplaced.cgu");
    assert_eq!(bodies[0].stage(), LlvmStage::NoLtoOptimized);
    assert_eq!(bodies[0].evidence().capture_id(), reference.capture_id());
    assert_eq!(bodies[0].evidence().artifact(), ArtifactId::new(5));
    assert_eq!(bodies[0].evidence().range(), ByteRange::new(8, 4).unwrap());
}

#[test]
fn all_modules_contribute_bodies_in_module_order_with_alias_provenance() {
    let fixture = TestStore::new();
    let reference = publish_modules(
        &fixture,
        "entry",
        vec![
            module(
                0,
                "z.cgu",
                vec![definition("entry", 0, LlvmDefinitionKind::Function)],
            ), //
            module(
                1,
                "unsupported.cgu",
                vec![definition("entry", 0, LlvmDefinitionKind::Ifunc)],
            ), //
            module(
                2,
                "a.cgu",
                vec![
                    definition("body", 12, LlvmDefinitionKind::Function), //
                    alias("middle", "body"),                              //
                    alias("entry", "middle"),                             //
                ],
            ), //
        ],
    );

    let LlvmEvidence::Available(bodies) = llvm_evidence(&fixture.store, &reference).unwrap() else {
        panic!("both supported modules have exact bodies");
    };

    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0].compiler_module(), "a.cgu");
    assert_eq!(bodies[0].raw_symbol(), "body");
    assert_eq!(bodies[0].aliases(), &["entry", "middle"]);
    assert_eq!(bodies[0].evidence().range(), ByteRange::new(12, 4).unwrap());
    assert_eq!(bodies[1].compiler_module(), "z.cgu");
    assert_eq!(bodies[1].raw_symbol(), "entry");
    assert!(bodies[1].aliases().is_empty());
}

#[test]
fn aliases_never_resolve_through_another_module() {
    let fixture = TestStore::new();
    let reference = publish_modules(
        &fixture,
        "entry",
        vec![
            module(0, "alias.cgu", vec![alias("entry", "body")]), //
            module(
                1,
                "body.cgu",
                vec![definition("body", 0, LlvmDefinitionKind::Function)],
            ), //
        ],
    );

    assert_eq!(
        llvm_evidence(&fixture.store, &reference).unwrap(),
        LlvmEvidence::NoExactDefinition
    );
}

#[test]
fn missing_exact_definitions_do_not_use_nearby_names() {
    for definitions in [
        Vec::new(), // A declaration has no indexed definition.
        // A display name is not identity.
        vec![definition("display", 0, LlvmDefinitionKind::Function)], //
        // A substring is not identity.
        vec![definition("entry.extra", 0, LlvmDefinitionKind::Function)], //
        vec![alias("entry", "missing")], // A direct target can lack a standalone definition.
    ] {
        let fixture = TestStore::new();
        let reference = publish_modules(&fixture, "entry", vec![module(0, "cgu", definitions)]);

        assert_eq!(
            llvm_evidence(&fixture.store, &reference).unwrap(),
            LlvmEvidence::NoExactDefinition
        );
    }
}

#[test]
fn unsupported_aliases_preserve_typed_unavailability() {
    for kind in [
        LlvmDefinitionKind::ExpressionAlias,
        LlvmDefinitionKind::Ifunc,
    ] {
        let fixture = TestStore::new();
        let reference = publish_modules(
            &fixture,
            "entry",
            vec![
                module(0, "absent.cgu", Vec::new()), //
                module(
                    1,
                    "alias.cgu",
                    vec![
                        alias("entry", "unsupported"),
                        definition("unsupported", 4, kind),
                    ],
                ), //
            ],
        );

        assert_eq!(
            llvm_evidence(&fixture.store, &reference).unwrap(),
            LlvmEvidence::UnsupportedAlias
        );
    }
}

#[test]
fn alias_cycles_are_errors_even_with_a_valid_body_in_another_module() {
    for aliases in [
        vec![alias("entry", "entry")], // A self-cycle.
        vec![alias("entry", "middle"), alias("middle", "entry")], // A longer cycle.
    ] {
        let fixture = TestStore::new();
        let reference = publish_modules(
            &fixture,
            "entry",
            vec![
                module(
                    0,
                    "body.cgu",
                    vec![definition("entry", 0, LlvmDefinitionKind::Function)],
                ), //
                module(1, "cycle.cgu", aliases), //
            ],
        );

        let error = llvm_evidence(&fixture.store, &reference).unwrap_err();

        assert!(matches!(
            error,
            Error::AliasCycle { reference: actual, compiler_module, raw_symbol }
                if actual == reference && compiler_module == "cycle.cgu" && raw_symbol == "entry"
        ));
    }
}

#[test]
fn unsupported_collection_preserves_the_configuration_reason() {
    for reason in [
        UnsupportedLlvmConfiguration::Incremental,        //
        UnsupportedLlvmConfiguration::CrossCrateThinLto,  //
        UnsupportedLlvmConfiguration::FatLto,             //
        UnsupportedLlvmConfiguration::LinkerPluginLto,    //
        UnsupportedLlvmConfiguration::OtherBackend,       //
        UnsupportedLlvmConfiguration::UnverifiedCompiler, //
    ] {
        let fixture = TestStore::new();
        let id = fixture.publish_evidence(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
            vec![instance("fixture", "display", "display", "entry")],
            Vec::new(),
            LlvmCollection::NotCaptured(reason),
        );

        assert_eq!(
            llvm_evidence(&fixture.store, &InstanceRef::new(id, 0)).unwrap(),
            LlvmEvidence::NotCaptured(reason)
        );
    }
}

#[test]
fn llvm_reads_stay_within_the_reference_capture() {
    let fixture = TestStore::new();
    let reference = publish_modules(
        &fixture,
        "entry",
        vec![module(
            0,
            "cgu",
            vec![definition("entry", 0, LlvmDefinitionKind::Function)],
        )],
    );
    let other = fixture.publish_evidence(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx",
        vec![instance("fixture", "display", "display", "entry")],
        vec![(
            ArtifactRecord::new(ArtifactId::new(0), ArtifactKind::Llvm, 0),
            b"",
        )],
        LlvmCollection::Collected(vec![module(0, "fixture.0", Vec::new())]),
    );

    assert!(matches!(
        llvm_evidence(&fixture.store, &reference).unwrap(),
        LlvmEvidence::Available(_)
    ));
    assert_eq!(
        llvm_evidence(&fixture.store, &InstanceRef::new(other, 0)).unwrap(),
        LlvmEvidence::NoExactDefinition
    );
}

#[test]
fn a_truncated_module_is_a_store_error_even_without_an_exact_match() {
    let fixture = TestStore::new();
    let reference = publish_modules(&fixture, "entry", vec![module(0, "empty.cgu", Vec::new())]);
    let artifact = ArtifactRecord::new(ArtifactId::new(0), ArtifactKind::Llvm, 32);
    let path = fixture
        .temporary
        .path()
        .join(".optic/store/captures")
        .join(reference.capture_id().as_str())
        .join(artifact.file_name());
    std::fs::write(&path, b"truncated").unwrap();

    let error = llvm_evidence(&fixture.store, &reference).unwrap_err();

    assert!(matches!(error, Error::Store { ref source }
        if source.to_string().contains(&path.display().to_string())));
}
