//! Exercises the packaged application API from an independent Cargo package.
//!
//! The installation script supplies a copied workspace and an isolated child environment. This
//! consumer uses only public exports and leaves durable evidence access to the application API.

use std::env;
use std::fs;

use optic::BuildRequest;
use optic::CaptureOutcome;
use optic::CapturePolicy;
use optic::CargoTarget;
use optic::EvidenceRange;
use optic::FoundInstance;
use optic::LlvmEvidence;
use optic::Optic;
use optic::SourceEvidence;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = env::current_dir()?;
    let optic = Optic::open(&workspace)?;
    let request = BuildRequest::new("default_tracking_fixture", CargoTarget::Library, "release")?;
    assert!(optic.list_captures()?.is_empty());

    let captured = optic.capture(&request, CapturePolicy::Reuse)?;
    assert!(matches!(captured, CaptureOutcome::Captured(_)));
    let original = captured.into_record();
    assert_eq!(optic.list_captures()?, [original.clone()]);

    let found = optic.find(
        original.id(),
        "default_tracking_fixture::captured_value",
        10,
    )?;
    assert_eq!(found.instances().len(), 1);
    let instance = &found.instances()[0];
    assert_eq!(instance.reference().capture_id(), original.id());
    let parsed = instance
        .reference()
        .to_string()
        .parse::<optic::InstanceRef>()?;
    assert_eq!(&parsed, instance.reference());
    let exact = optic.find(original.id(), instance.record().raw_symbol(), 10)?;
    assert_eq!(exact.instances()[0].reference(), instance.reference());

    let reused = optic.capture(&request, CapturePolicy::Reuse)?;
    assert!(matches!(reused, CaptureOutcome::Reused(_)));
    assert_eq!(reused.record(), &original);
    assert_eq!(optic.list_captures()?, [original.clone()]);

    let SourceEvidence::Available {
        evidence: original_source,
        ..
    } = optic.source(instance.reference())?
    else {
        panic!("the local fixture function must have captured source");
    };
    let source = copy(&optic, &original_source)?;
    assert_eq!(source, b"pub fn captured_value() -> u64 {\n    42\n}");
    let llvm = exact_llvm(&optic, instance)?;

    let path = workspace.join("src/lib.rs");
    let edited = fs::read_to_string(&path)?.replace("    42", "    12345");
    fs::write(path, edited)?;
    assert_eq!(copy(&optic, &original_source)?, source);
    assert_eq!(exact_llvm(&optic, instance)?, llvm);

    let changed = optic.capture(&request, CapturePolicy::Reuse)?;
    assert!(matches!(changed, CaptureOutcome::Captured(_)));
    assert_ne!(changed.record().id(), original.id());
    let changed_found = optic.find(changed.record().id(), "captured_value", 10)?;
    assert_eq!(changed_found.instances().len(), 1);
    let SourceEvidence::Available { evidence, .. } =
        optic.source(changed_found.instances()[0].reference())?
    else {
        panic!("the edited function must have captured source");
    };
    assert_eq!(
        copy(&optic, &evidence)?,
        b"pub fn captured_value() -> u64 {\n    12345\n}"
    );
    assert_ne!(exact_llvm(&optic, &changed_found.instances()[0])?, llvm);
    assert_eq!(copy(&optic, &original_source)?, source);
    assert_eq!(exact_llvm(&optic, instance)?, llvm);

    let fresh = optic.capture(&request, CapturePolicy::Fresh)?;
    assert!(matches!(fresh, CaptureOutcome::Captured(_)));
    assert_ne!(fresh.record().id(), changed.record().id());
    assert_ne!(fresh.record().id(), original.id());
    let reused = optic.capture(&request, CapturePolicy::Reuse)?;
    assert!(matches!(reused, CaptureOutcome::Reused(_)));
    assert_eq!(reused.record(), fresh.record());
    assert_eq!(optic.list_captures()?.len(), 3);
    println!("Packaged API journey passed: {}", fresh.record().id());

    Ok(())
}

fn copy(optic: &Optic, evidence: &EvidenceRange) -> Result<Vec<u8>, optic::Error> {
    let mut bytes = Vec::new();
    optic.copy_evidence(evidence, &mut bytes)?;

    Ok(bytes)
}

fn exact_llvm(optic: &Optic, instance: &FoundInstance) -> Result<Vec<Vec<u8>>, optic::Error> {
    let LlvmEvidence::Available(bodies) = optic.llvm(instance.reference())? else {
        panic!("the release fixture must have exact LLVM bodies");
    };
    assert!(!bodies.is_empty());
    let mut result = Vec::new();

    for body in bodies {
        assert_eq!(
            body.evidence().capture_id(),
            instance.reference().capture_id()
        );
        assert_eq!(body.raw_symbol(), instance.record().raw_symbol());
        assert!(body.aliases().is_empty());
        let bytes = copy(optic, body.evidence())?;
        let text = std::str::from_utf8(&bytes).expect("llvm-dis output must be UTF-8");
        assert!(text.starts_with("define "));
        assert!(text.contains(&format!("@{}(", body.raw_symbol())));
        result.push(bytes);
    }

    Ok(result)
}
