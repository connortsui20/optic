//! Exercises captured source and optimized LLVM through the public API.
//!
//! Compiler-loaded bytes and typed availability cross real process boundaries. Compiler tests own
//! the retained bitcode-stage proof, and evidence unit tests own index and alias syntax cases.

mod common;

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use cargo_optic_test_support::assert_success;
use cargo_optic_test_support::run;
use optic::BuildRequest;
use optic::CaptureId;
use optic::CaptureOutcome;
use optic::CapturePolicy;
use optic::CaptureRecord;
use optic::CargoTarget;
use optic::EvidenceRange;
use optic::InstanceRef;
use optic::LlvmEvidence;
use optic::LlvmStage;
use optic::Optic;
use optic::SourceEvidence;
use optic::SourceUnavailable;
use optic::UnsupportedLlvmConfiguration;

use common::workspace_in_child;

fn request(profile: &str) -> BuildRequest {
    BuildRequest::new(
        "show_fixture",
        CargoTarget::Binary("show_fixture".to_owned()),
        profile,
    )
    .unwrap()
}

#[track_caller]
fn capture(optic: &Optic, profile: &str) -> CaptureRecord {
    let outcome = optic
        .capture(&request(profile), CapturePolicy::Reuse)
        .unwrap();
    assert!(matches!(outcome, CaptureOutcome::Captured(_)));

    outcome.into_record()
}

#[track_caller]
fn reference(optic: &Optic, capture: &CaptureId, query: &str) -> InstanceRef {
    let found = optic.find(capture, query, 100).unwrap();
    assert_eq!(found.instances().len(), 1, "{query}: {found:?}");

    found.instances()[0].reference().clone()
}

fn copy(optic: &Optic, evidence: &EvidenceRange) -> Vec<u8> {
    let mut bytes = Vec::new();
    optic.copy_evidence(evidence, &mut bytes).unwrap();

    bytes
}

#[test]
fn shows_whole_functions_and_methods() {
    let Some(workspace) = workspace_in_child("shows_whole_functions_and_methods", "show") else {
        return;
    };
    let optic = Optic::open(&workspace).unwrap();
    let capture = capture(&optic, "release");

    for (query, expected_file, source_file) in [
        (
            "show_fixture::source_items::ordinary",
            "ordinary.txt",
            "source_items.rs",
        ), // Whole function.
        (
            "show_fixture::source_items::Widget::method",
            "method.txt",
            "source_items.rs",
        ), // Whole method.
    ] {
        let reference = reference(&optic, capture.id(), query);
        let SourceEvidence::Available {
            evidence,
            display_path,
            starting_line,
        } = optic.source(&reference).unwrap()
        else {
            panic!("expected whole source for {query}");
        };
        let expected = fs::read_to_string(workspace.join("expected").join(expected_file)).unwrap();
        let expected = expected.strip_suffix('\n').unwrap();
        let original = fs::read_to_string(workspace.join("src").join(source_file)).unwrap();
        let offset = original.find(expected).unwrap();

        assert_eq!(copy(&optic, &evidence), expected.as_bytes());
        assert_eq!(evidence.capture_id(), capture.id());
        assert!(display_path.ends_with(Path::new("src").join(source_file)));
        assert_eq!(
            starting_line,
            original[..offset]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count() as u64
                + 1
        );
    }
}

#[test]
fn generic_instances_share_source_and_keep_references_after_limiting() {
    let Some(workspace) = workspace_in_child(
        "generic_instances_share_source_and_keep_references_after_limiting",
        "show",
    ) else {
        return;
    };
    let optic = Optic::open(&workspace).unwrap();
    let capture = capture(&optic, "release");
    let query = "show_fixture::source_items::generic";
    let found = optic.find(capture.id(), query, 100).unwrap();
    let [first, second] = found.instances() else {
        panic!("expected two generic instances, got {found:?}");
    };
    assert_ne!(first.reference(), second.reference());
    assert_ne!(first.record().raw_symbol(), second.record().raw_symbol());
    let first_source = optic.source(first.reference()).unwrap();
    assert_eq!(first_source, optic.source(second.reference()).unwrap());
    let SourceEvidence::Available { evidence, .. } = first_source else {
        panic!("expected shared generic source");
    };
    let expected = fs::read_to_string(workspace.join("expected/generic.txt")).unwrap();
    assert_eq!(
        copy(&optic, &evidence),
        expected.strip_suffix('\n').unwrap().as_bytes()
    );

    let limited = optic.find(capture.id(), query, 1).unwrap();
    assert_eq!(limited.instances().len(), 1);
    let selected = &limited.instances()[0];
    let original = found
        .instances()
        .iter()
        .find(|instance| instance.record().raw_symbol() == selected.record().raw_symbol())
        .unwrap();
    assert_eq!(selected.reference(), original.reference());
    let exact = optic
        .find(capture.id(), selected.record().raw_symbol(), 100)
        .unwrap();
    assert_eq!(exact.instances().len(), 1);
    assert_eq!(exact.instances()[0].reference(), selected.reference());
}

#[test]
fn uses_normalized_loaded_source_with_unicode_bom_crlf_and_spaces() {
    let Some(workspace) = workspace_in_child(
        "uses_normalized_loaded_source_with_unicode_bom_crlf_and_spaces",
        "show",
    ) else {
        return;
    };
    let source = workspace.join("src/source with spaces.rs");
    let normalized = fs::read_to_string(&source).unwrap();
    assert!(normalized.contains("π 雪 🦀"));
    fs::write(
        &source,
        format!("\u{feff}{}", normalized.replace('\n', "\r\n")),
    )
    .unwrap();
    let optic = Optic::open(&workspace).unwrap();
    let capture = capture(&optic, "release");
    let reference = reference(
        &optic,
        capture.id(),
        "show_fixture::spaced::unicode_function",
    );
    let SourceEvidence::Available {
        evidence,
        display_path,
        starting_line,
    } = optic.source(&reference).unwrap()
    else {
        panic!("expected compiler-normalized source");
    };
    let expected = fs::read_to_string(workspace.join("expected/unicode.txt")).unwrap();
    let expected = expected.strip_suffix('\n').unwrap();
    let offset = normalized.find(expected).unwrap();

    assert_eq!(copy(&optic, &evidence), expected.as_bytes());
    assert_eq!(evidence.range().start(), offset as u64);
    assert_eq!(
        starting_line,
        normalized[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u64
            + 1
    );
    assert!(display_path.ends_with("src/source with spaces.rs"));
}

#[test]
fn reports_nonlocal_generated_and_outside_package_source() {
    let Some(workspace) = workspace_in_child(
        "reports_nonlocal_generated_and_outside_package_source",
        "show",
    ) else {
        return;
    };
    let outside = workspace.parent().unwrap().join("outside.rs");
    fs::write(outside, "#[inline(never)]\npub fn outside_function(value: u64) -> u64 { std::hint::black_box(value.wrapping_add(13)) }\n").unwrap();
    let main = workspace.join("src/main.rs");
    let contents = fs::read_to_string(&main).unwrap();
    fs::write(
        main,
        contents
            .replace(
                "mod source_items;",
                "mod source_items;\n#[path = \"../../outside.rs\"]\nmod outside;",
            )
            .replace(
                "source_items::ordinary(value),",
                "source_items::ordinary(value),\n        outside::outside_function(value),",
            ),
    )
    .unwrap();
    let optic = Optic::open(&workspace).unwrap();
    let capture = capture(&optic, "release");

    for (query, reason) in [
        (
            "show_dependency::external_generic",
            SourceUnavailable::Nonlocal,
        ), // External generic definition.
        (
            "show_fixture::source_items::generated",
            SourceUnavailable::Generated,
        ), // Macro-generated function.
        (
            "show_fixture::outside::outside_function",
            SourceUnavailable::OutsidePackage,
        ), // Loaded file outside the root.
    ] {
        let reference = reference(&optic, capture.id(), query);
        assert_eq!(
            optic.source(&reference).unwrap(),
            SourceEvidence::Unavailable(reason)
        );
    }
}

#[test]
fn returns_exact_optimized_bodies_for_both_supported_stages() {
    let Some(workspace) = workspace_in_child(
        "returns_exact_optimized_bodies_for_both_supported_stages",
        "show",
    ) else {
        return;
    };
    let optic = Optic::open(&workspace).unwrap();

    for (profile, stage) in [
        ("release", LlvmStage::LocalThinLtoPostPassManager), // Default release with multiple CGUs.
        ("no-lto", LlvmStage::NoLtoOptimized),               // Explicitly disabled LTO.
    ] {
        let capture = capture(&optic, profile);
        let found = optic
            .find(capture.id(), "show_fixture::source_items::ordinary", 100)
            .unwrap();
        assert_eq!(found.instances().len(), 1);
        let instance = &found.instances()[0];
        let LlvmEvidence::Available(bodies) = optic.llvm(instance.reference()).unwrap() else {
            panic!("expected optimized LLVM for {profile}");
        };
        assert!(!bodies.is_empty());

        for body in &bodies {
            assert_eq!(body.stage(), stage);
            assert!(!body.compiler_module().is_empty());
            assert_eq!(body.evidence().capture_id(), capture.id());
            assert!(body.aliases().is_empty());
            assert_eq!(body.raw_symbol(), instance.record().raw_symbol());
            let llvm = String::from_utf8(copy(&optic, body.evidence())).unwrap();
            assert!(llvm.starts_with("define "), "{llvm}");
            assert!(llvm.contains(&format!("@{}(", body.raw_symbol())), "{llvm}");
            assert!(llvm.contains("shl i64"), "{llvm}");
            assert!(!llvm.contains("alloca"), "{llvm}");
        }

        let reused = optic
            .capture(&request(profile), CapturePolicy::Reuse)
            .unwrap();
        assert!(matches!(reused, CaptureOutcome::Reused(_)));
        assert_eq!(reused.record(), &capture);
    }
}

#[test]
fn reads_stored_source_and_llvm_after_checkout_edits() {
    let Some(workspace) =
        workspace_in_child("reads_stored_source_and_llvm_after_checkout_edits", "show")
    else {
        return;
    };
    let optic = Optic::open(&workspace).unwrap();
    let capture = capture(&optic, "release");
    let reference = reference(&optic, capture.id(), "show_fixture::source_items::ordinary");
    let source = optic.source(&reference).unwrap();
    let SourceEvidence::Available { evidence, .. } = &source else {
        panic!("expected stored source");
    };
    let source_bytes = copy(&optic, evidence);
    let llvm = optic.llvm(&reference).unwrap();
    let LlvmEvidence::Available(bodies) = &llvm else {
        panic!("expected stored LLVM");
    };
    let llvm_bytes = bodies
        .iter()
        .map(|body| copy(&optic, body.evidence()))
        .collect::<Vec<_>>();
    assert!(!llvm_bytes.is_empty());
    fs::write(workspace.join("src/source_items.rs"), "pub fn broken( {\n").unwrap();
    let cargo_home = std::path::PathBuf::from(env::var_os("CARGO_HOME").unwrap());
    let drivers = cargo_home.join("optic/drivers");
    assert!(drivers.is_dir());
    fs::rename(&drivers, cargo_home.join("optic/retained-drivers")).unwrap();

    assert_eq!(optic.source(&reference).unwrap(), source);
    assert_eq!(copy(&optic, evidence), source_bytes);
    assert_eq!(optic.llvm(&reference).unwrap(), llvm);
    assert_eq!(
        bodies
            .iter()
            .map(|body| copy(&optic, body.evidence()))
            .collect::<Vec<_>>(),
        llvm_bytes
    );
    assert!(!drivers.exists());
    assert_eq!(optic.list_captures().unwrap(), [capture]);
}

#[test]
fn unsupported_llvm_preserves_source_capture_and_warm_reuse() {
    for profile in ["incremental", "cross-thin", "fat"] {
        let workspace = TestWorkspace::new("show");
        let mut command = Command::new(env::current_exe().unwrap());
        workspace.apply(&mut command);
        command
            .args(["--exact", "unsupported_llvm_child", "--nocapture"])
            .env("OPTIC_TEST_PROFILE", profile);
        let output = run(&mut command);
        assert_success(&command, &output);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            stderr
                .lines()
                .filter(|line| line.starts_with("warning:")
                    && line.to_ascii_lowercase().contains("llvm"))
                .count(),
            2,
            "{profile}: {stderr}"
        );
    }
}

#[test]
fn unsupported_llvm_child() {
    let Ok(profile) = env::var("OPTIC_TEST_PROFILE") else {
        return;
    };
    let reason = match profile.as_str() {
        "incremental" => UnsupportedLlvmConfiguration::Incremental,
        "cross-thin" => UnsupportedLlvmConfiguration::CrossCrateThinLto,
        "fat" => UnsupportedLlvmConfiguration::FatLto,
        _ => panic!("unknown profile {profile}"),
    };
    let optic = Optic::open(&env::current_dir().unwrap()).unwrap();
    let capture = capture(&optic, &profile);
    let reference = reference(&optic, capture.id(), "show_fixture::source_items::ordinary");
    assert!(matches!(
        optic.source(&reference).unwrap(),
        SourceEvidence::Available { .. }
    ));
    assert_eq!(
        optic.llvm(&reference).unwrap(),
        LlvmEvidence::NotCaptured(reason)
    );
    let reused = optic
        .capture(&request(&profile), CapturePolicy::Reuse)
        .unwrap();
    assert!(matches!(reused, CaptureOutcome::Reused(_)));
    assert_eq!(reused.record(), &capture);
    assert_eq!(optic.list_captures().unwrap(), [capture]);
    assert_eq!(
        optic.llvm(&reference).unwrap(),
        LlvmEvidence::NotCaptured(reason)
    );
}
