//! Protects durable record validation.
//!
//! Constructors and deserializers must reject the same malformed fields.

mod evidence;

use std::path::PathBuf;

use crate::AnalysisToken;
use crate::BuildRecord;
use crate::CaptureAnalysis;
use crate::CaptureId;
use crate::CaptureKey;
use crate::CaptureRecord;
use crate::CargoArtifactRecord;
use crate::CargoTargetKind;
use crate::CompilerIdentity;
use crate::DefinitionRecord;
use crate::InstanceManifest;
use crate::InstanceRecord;
use crate::LlvmCollection;
use crate::LlvmLto;
use crate::LlvmProvenance;
use crate::PlacementRecord;
use crate::SourceAvailability;
use crate::SourceUnavailable;
use crate::TargetRecord;
use crate::UnsupportedLlvmConfiguration;

fn capture_id() -> CaptureId {
    "zyxwvutsrqponmlkzyxwvutsrqponmlk"
        .parse()
        .expect("the fixture capture ID is valid")
}

fn compiler_identity() -> CompilerIdentity {
    let root = std::env::current_dir().expect("the test invocation directory is available");

    CompilerIdentity::new(
        root.join("toolchain/bin/rustc"),
        "1.99.0-nightly",
        "0123456789abcdef0123456789abcdef01234567",
        "aarch64-apple-darwin",
        root.join("toolchain"),
    )
    .expect("the fixture compiler identity is valid")
}

fn record() -> CaptureRecord {
    let target =
        TargetRecord::new("example", CargoTargetKind::Lib).expect("the fixture target is valid");
    let build = BuildRecord::new(
        "example",
        "0.1.0",
        target,
        "release",
        PathBuf::from("cargo"),
        std::env::current_dir().expect("the test invocation directory is available"),
        vec!["rustc".to_owned()],
    )
    .expect("the fixture build is valid");

    CaptureRecord::new(capture_id(), 1_000, build, compiler_identity(), analysis())
}

fn cargo_artifact() -> cargo_metadata::Artifact {
    serde_json::from_value(serde_json::json!({
        "package_id": "path+file:///workspace#example@0.1.0",
        "manifest_path": "/workspace/Cargo.toml",
        "target": {
            "name": "example",
            "kind": ["lib", "rlib"],
            "crate_types": ["lib", "rlib"],
            "required-features": ["alpha", "beta"],
            "src_path": "/workspace/src/lib.rs",
            "edition": "2024",
            "doc": true, "doctest": true, "test": true
        },
        "profile": {
            "opt_level": "s", "debuginfo": 1,
            "debug_assertions": false, "overflow_checks": true, "test": false
        },
        "features": ["alpha", "beta"],
        "filenames": ["/workspace/target/libexample.rlib", "/workspace/target/libexample.rmeta"],
        "executable": null,
        "fresh": false
    }))
    .expect("the Cargo artifact fixture is valid")
}

fn analysis() -> CaptureAnalysis {
    CaptureAnalysis::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .expect("the fixture key is valid"),
        "0123456789ab4def8123456789abcdef"
            .parse()
            .expect("the fixture token is valid"),
        CargoArtifactRecord::new(cargo_artifact()).expect("the fixture artifact is valid"),
    )
}

fn instance() -> InstanceRecord {
    let definition = DefinitionRecord::new("example", "example::kernel")
        .expect("the fixture definition is valid");
    let placement = PlacementRecord::new("example-cgu.0", "External", "Default", false, 17)
        .expect("the fixture placement is valid");

    InstanceRecord::new(
        definition,
        "example::kernel::<u64>",
        "_RNvCexample6kernelm",
        vec![placement],
        SourceAvailability::Unavailable(SourceUnavailable::Nonlocal),
    )
    .expect("the fixture instance is valid")
}

fn manifest() -> InstanceManifest {
    InstanceManifest::new(
        capture_id(),
        vec![instance()],
        vec![],
        provenance(),
        LlvmCollection::NotCaptured(UnsupportedLlvmConfiguration::UnverifiedCompiler),
    )
    .expect("the fixture instance manifest is valid")
}

fn provenance() -> LlvmProvenance {
    LlvmProvenance::new(
        "llvm",
        Some("22.1.0".into()),
        "x86_64-unknown-linux-gnu",
        "3",
        LlvmLto::Off,
        false,
        false,
        16,
        1,
    )
    .unwrap()
}

#[track_caller]
fn assert_record_error(encoded: &str, expected: &str) {
    let error = serde_json::from_str::<CaptureRecord>(encoded)
        .expect_err("the invalid record must be rejected");

    assert!(
        error.to_string().contains(expected),
        "expected an error containing {expected:?}, got {error}"
    );
}

#[test]
fn rejects_noncanonical_capture_ids() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let noncanonical = encoded.replace(
        "zyxwvutsrqponmlkzyxwvutsrqponmlk",
        "cap_0123456789abcdef0123456789abcdef",
    );

    assert_record_error(&noncanonical, "capture ID must contain exactly 32");
}

#[test]
fn round_trips_a_valid_record() {
    let expected = record();
    let encoded = serde_json::to_vec(&expected).expect("the fixture record can be encoded");
    let actual = serde_json::from_slice::<CaptureRecord>(&encoded)
        .expect("the encoded fixture record can be read");

    assert_eq!(actual, expected);
}

#[test]
fn rejects_an_unknown_capture_format() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":4"#, r#""format_version":5"#);

    assert_record_error(
        &unsupported_version,
        "capture format version must be 4, got 5",
    );
}

#[test]
fn reports_the_previous_capture_format_before_its_missing_fields() {
    let mut previous = serde_json::to_value(record()).expect("the fixture record can be encoded");
    let previous = previous
        .as_object_mut()
        .expect("the capture fixture is an object");
    previous.insert("format_version".to_owned(), serde_json::Value::from(1));
    previous.remove("compiler");
    previous.remove("analysis");

    let error =
        serde_json::from_value::<CaptureRecord>(serde_json::Value::Object(previous.clone()))
            .expect_err("the previous capture format must be rejected");

    assert_eq!(error.to_string(), "capture format version must be 4, got 1");
}

#[test]
fn round_trips_a_manifest_with_no_collected_llvm_modules() {
    let expected = manifest();
    let encoded = serde_json::to_string(&expected).expect("the fixture manifest can be encoded");
    let actual = serde_json::from_str::<InstanceManifest>(&encoded)
        .expect("the encoded fixture manifest can be read");

    assert_eq!(actual, expected);
    assert_eq!(actual.format_version(), 4);
    assert_eq!(actual.capture_id(), record().id());
    assert!(!encoded.contains("body"));
}

#[test]
fn rejects_an_unknown_instance_manifest_format() {
    let encoded = serde_json::to_string(&manifest()).expect("the fixture manifest can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":4"#, r#""format_version":5"#);
    let error = serde_json::from_str::<InstanceManifest>(&unsupported_version)
        .expect_err("the unsupported manifest must be rejected");

    assert_eq!(error.to_string(), "capture format version must be 4, got 5");
}

#[test]
fn rejects_a_malformed_instance_manifest() {
    let encoded = serde_json::to_string(&manifest()).expect("the fixture manifest can be encoded");
    let mut malformed =
        serde_json::from_str::<serde_json::Value>(&encoded).expect("the fixture JSON is valid");
    malformed["instances"][0]["placements"] = serde_json::Value::Array(Vec::new());
    let malformed =
        serde_json::to_string(&malformed).expect("the modified fixture manifest can be encoded");
    let error = serde_json::from_str::<InstanceManifest>(&malformed)
        .expect_err("the malformed manifest must be rejected");

    assert!(
        error
            .to_string()
            .contains("instance placements must contain a valid value")
    );
}

#[test]
fn rejects_duplicate_codegen_unit_placements() {
    let placement = PlacementRecord::new("example-cgu.0", "External", "Default", false, 17)
        .expect("the fixture placement is valid");
    let definition = DefinitionRecord::new("example", "example::kernel")
        .expect("the fixture definition is valid");
    let error = InstanceRecord::new(
        definition,
        "example::kernel::<u64>",
        "_RNvCexample6kernelm",
        vec![placement.clone(), placement],
        SourceAvailability::Unavailable(SourceUnavailable::Nonlocal),
    )
    .expect_err("duplicate codegen units must be rejected");

    assert!(error.to_string().contains("duplicate codegen unit"));
}

#[test]
fn rejects_duplicate_instance_identities() {
    let duplicate = instance();
    let error = InstanceManifest::new(
        capture_id(),
        vec![duplicate.clone(), duplicate],
        vec![],
        provenance(),
        LlvmCollection::Collected(vec![]),
    )
    .expect_err("duplicate instance identities must be rejected");

    assert!(error.to_string().contains("duplicate instance"));
}

#[test]
fn rejects_each_invalid_compiler_path() {
    let root = std::env::current_dir().expect("the test invocation directory is available");
    let cases = [
        (PathBuf::new(), "rustc path"),            // Empty rustc path.
        (PathBuf::from("rustc"), "relative path"), // Relative rustc path.
        (root.join("toolchain/./rustc"), "not lexically normalized"), // Dot rustc path.
        (root.join("toolchain/../rustc"), "not lexically normalized"), // Non-normal rustc path.
    ];

    for (rustc, expected) in cases {
        let error = CompilerIdentity::new(
            rustc,
            "1.99.0-nightly",
            "custom-commit",
            "aarch64-apple-darwin",
            root.join("toolchain"),
        )
        .expect_err("the invalid compiler identity must be rejected");

        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn rejects_each_invalid_compiler_sysroot() {
    let root = std::env::current_dir().expect("the test invocation directory is available");
    let rustc = root.join("toolchain/bin/rustc");
    let sysroots = [
        PathBuf::new(),                    // Empty sysroot path.
        PathBuf::from("toolchain"),        // Relative sysroot path.
        root.join("toolchain/../sysroot"), // Non-normal sysroot path.
    ];

    for sysroot in sysroots {
        let error = CompilerIdentity::new(
            rustc.clone(),
            "1.99.0-nightly",
            "custom-commit",
            "aarch64-apple-darwin",
            sysroot,
        )
        .expect_err("the invalid compiler sysroot must be rejected");

        assert!(error.to_string().contains("rustc sysroot"));
    }
}

#[test]
fn deserialization_rejects_each_empty_compiler_text_field() {
    let encoded = serde_json::to_value(record()).expect("the fixture record can be encoded");
    let cases = [
        ("/compiler/release", "rustc release"), // Release string.
        ("/compiler/commit_hash", "rustc commit hash"), // Commit hash.
        ("/compiler/host", "rustc host"),       // Host triple.
    ];

    for (pointer, field) in cases {
        let mut invalid = encoded.clone();
        *invalid
            .pointer_mut(pointer)
            .expect("the compiler fixture field exists") = serde_json::Value::String(String::new());
        let error = serde_json::from_value::<CaptureRecord>(invalid)
            .expect_err("the empty compiler field must be rejected");

        assert!(error.to_string().contains(field));
    }
}

#[test]
fn deserialization_rejects_each_empty_instance_text_field() {
    let encoded = serde_json::to_value(manifest()).expect("the fixture manifest can be encoded");
    let cases = [
        (
            "/instances/0/definition/crate_name",
            "definition crate name",
        ), // Definition crate.
        ("/instances/0/definition/definition_path", "definition path"), // Definition path.
        ("/instances/0/display_name", "instance display name"),         // Display name.
        ("/instances/0/raw_symbol", "instance raw symbol"),             // Raw symbol.
        ("/instances/0/placements/0/codegen_unit", "codegen unit"),     // Codegen unit.
        ("/instances/0/placements/0/linkage", "placement linkage"),     // Linkage.
        (
            "/instances/0/placements/0/visibility",
            "placement visibility",
        ), // Visibility.
    ];

    for (pointer, field) in cases {
        let mut invalid = encoded.clone();
        *invalid
            .pointer_mut(pointer)
            .expect("the instance fixture field exists") = serde_json::Value::String(String::new());
        let error = serde_json::from_value::<InstanceManifest>(invalid)
            .expect_err("the empty instance field must be rejected");

        assert!(error.to_string().contains(field));
    }
}

#[test]
fn deserialization_rejects_unknown_evidence_fields() {
    let encoded = serde_json::to_value(manifest()).expect("the fixture manifest can be encoded");
    let pointers = [
        "",                          // Manifest.
        "/instances/0",              // Instance.
        "/instances/0/definition",   // Definition.
        "/instances/0/placements/0", // Placement.
    ];

    for pointer in pointers {
        let mut invalid = encoded.clone();
        invalid
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .expect("the evidence fixture object exists")
            .insert("unknown".to_owned(), serde_json::Value::Bool(true));
        let error = serde_json::from_value::<InstanceManifest>(invalid)
            .expect_err("the unknown evidence field must be rejected");

        assert!(error.to_string().contains("unknown field `unknown`"));
    }

    let mut invalid = serde_json::to_value(record()).expect("the fixture record can be encoded");
    invalid["compiler"]["unknown"] = serde_json::Value::Bool(true);
    let error = serde_json::from_value::<CaptureRecord>(invalid)
        .expect_err("the unknown compiler field must be rejected");

    assert!(error.to_string().contains("unknown field `unknown`"));
}

#[test]
fn rejects_an_unknown_target_kind() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let invalid_target = encoded.replace(r#""kind":"lib""#, r#""kind":"test""#);

    assert_record_error(&invalid_target, "unknown variant `test`");
}

#[test]
fn rejects_each_empty_text_field() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let cases = [
        (r#""package":"example""#, r#""package":"""#, "package name"), // Package name.
        (
            r#""package_version":"0.1.0""#,
            r#""package_version":"""#,
            "package version",
        ), // Package version.
        (r#""name":"example""#, r#""name":"""#, "target name"),        // Target name.
        (r#""profile":"release""#, r#""profile":"""#, "profile"),      // Profile name.
        (
            r#""cargo_program":"cargo""#,
            r#""cargo_program":"""#,
            "Cargo program",
        ), // Cargo program.
    ];

    for (original, replacement, field) in cases {
        let empty_field = encoded.replace(original, replacement);

        assert_record_error(&empty_field, &format!("{field} must contain a valid value"));
    }
}

#[test]
fn rejects_an_empty_cargo_argument_list() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let empty_arguments =
        encoded.replace(r#""cargo_arguments":["rustc"]"#, r#""cargo_arguments":[]"#);

    assert_record_error(
        &empty_arguments,
        "Cargo arguments must contain a valid value",
    );
}

#[test]
fn rejects_each_invalid_invocation_directory() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let cases = [
        ("", "an empty path"),    // Empty path.
        (".", "a relative path"), // Relative path.
    ];

    for (directory, actual) in cases {
        let mut invalid =
            serde_json::from_str::<serde_json::Value>(&encoded).expect("the fixture JSON is valid");
        invalid["build"]["invocation_directory"] = serde_json::Value::String(directory.to_owned());
        let invalid =
            serde_json::to_string(&invalid).expect("the modified fixture record can be encoded");

        assert_record_error(&invalid, actual);
    }
}

#[test]
fn rejects_unknown_record_fields() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let unknown_field = encoded.replacen('{', r#"{"unknown":true,"#, 1);

    assert_record_error(&unknown_field, "unknown field `unknown`");
}

#[test]
fn validates_request_keys_at_parse_and_deserialization() {
    let valid = analysis().request_key().to_string();
    assert_eq!(valid.parse::<CaptureKey>().unwrap().as_str(), valid);

    for invalid in [
        String::new(),  // Empty digest.
        "a".repeat(63), // Short digest.
        "a".repeat(65), // Long digest.
        "A".repeat(64), // Uppercase digest.
        "g".repeat(64), // Non-hexadecimal digest.
        "é".repeat(32), // Non-ASCII digest.
    ] {
        assert!(invalid.parse::<CaptureKey>().is_err(), "{invalid:?}");
        assert!(serde_json::from_value::<CaptureKey>(invalid.into()).is_err());
    }
}

#[test]
fn generates_and_validates_analysis_tokens() {
    let first = AnalysisToken::generate();
    let second = AnalysisToken::generate();
    assert_ne!(first, second);
    assert_eq!(first.as_str().len(), 32);
    assert_eq!(first.as_str().parse::<AnalysisToken>().unwrap(), first);
    assert_eq!(
        serde_json::from_value::<AnalysisToken>(serde_json::to_value(&first).unwrap()).unwrap(),
        first
    );

    for invalid in [
        "",                                     // Empty token.
        "0123456789ab4def8123456789abcde",      // Short token.
        "01234567-89ab-4def-8123-456789abcdef", // Hyphenated UUID.
        "0123456789AB4DEF8123456789ABCDEF",     // Uppercase UUID.
        "0123456789ab1def8123456789abcdef",     // Wrong UUID version.
        "0123456789ab4def7123456789abcdef",     // Wrong UUID variant.
        "0123456789ab4def8123456789abcdeg",     // Non-hexadecimal digit.
    ] {
        assert!(invalid.parse::<AnalysisToken>().is_err(), "{invalid:?}");
        assert!(serde_json::from_value::<AnalysisToken>(invalid.into()).is_err());
    }
}

#[test]
fn normalizes_cargo_observations_without_changing_profile_settings() {
    let artifact = cargo_artifact();
    let expected = CargoArtifactRecord::new(artifact.clone()).unwrap();
    let mut reordered = artifact;
    reordered.fresh = true;
    reordered.features.reverse();
    reordered.filenames.reverse();
    reordered.target.kind.reverse();
    reordered.target.crate_types.reverse();
    reordered.target.required_features.reverse();

    assert_eq!(
        CargoArtifactRecord::new(reordered.clone()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::from_value::<CargoArtifactRecord>(serde_json::to_value(reordered).unwrap())
            .unwrap(),
        expected
    );
    assert!(!expected.artifact().fresh);
    assert_eq!(expected.artifact().profile.opt_level, "s");
    assert!(expected.artifact().profile.overflow_checks);
}

#[test]
fn validates_cargo_artifact_identity_and_paths() {
    let encoded = serde_json::to_value(cargo_artifact()).unwrap();
    let cases = [
        ("/package_id", serde_json::json!("")), // Missing package identity.
        ("/target/name", serde_json::json!("")), // Missing target name.
        ("/target/kind", serde_json::json!([])), // Missing target kinds.
        ("/target/crate_types", serde_json::json!([])), // Missing crate types.
        ("/target/kind", serde_json::json!([""])), // Empty target kind.
        ("/target/crate_types", serde_json::json!([""])), // Empty crate type.
        ("/profile/opt_level", serde_json::json!("")), // Missing optimization level.
        ("/manifest_path", serde_json::json!("Cargo.toml")), // Relative manifest path.
        ("/target/src_path", serde_json::json!("src/lib.rs")), // Relative source path.
        (
            "/target/src_path",
            serde_json::json!("/workspace/../lib.rs"),
        ), // Parent traversal.
        ("/filenames", serde_json::json!(["target/lib.rlib"])), // Relative output path.
        ("/executable", serde_json::json!("target/example")), // Relative executable path.
        ("/features", serde_json::json!([""])), // Empty feature.
        ("/target/required-features", serde_json::json!([""])), // Empty required feature.
    ];

    for (pointer, value) in cases {
        let mut invalid = encoded.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        let artifact = serde_json::from_value(invalid.clone()).unwrap();
        assert!(CargoArtifactRecord::new(artifact).is_err(), "{pointer}");
        assert!(
            serde_json::from_value::<CargoArtifactRecord>(invalid).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn accepts_empty_cargo_features_and_outputs() {
    let mut artifact = cargo_artifact();
    artifact.features.clear();
    artifact.target.required_features.clear();
    artifact.filenames.clear();

    assert!(CargoArtifactRecord::new(artifact).is_ok());
}

#[test]
fn requires_analysis_in_current_capture_records() {
    let mut encoded = serde_json::to_value(record()).unwrap();
    encoded.as_object_mut().unwrap().remove("analysis");

    assert_record_error(
        &encoded.to_string(),
        "analysis must contain a valid value, got no value",
    );
}
