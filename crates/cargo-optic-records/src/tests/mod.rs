//! Protects durable record validation.
//!
//! Constructors and deserializers must reject the same malformed fields.

mod evidence;

use std::fmt::Debug;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

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

/// Checks that a valid record survives conversion to and from a JSON value.
#[track_caller]
fn assert_json_round_trip<T>(expected: &T)
where
    T: Serialize + DeserializeOwned + Debug + PartialEq,
{
    let encoded = serde_json::to_value(expected).expect("the valid fixture can be encoded as JSON");
    let actual = serde_json::from_value::<T>(encoded)
        .expect("the JSON produced from the valid fixture can be read");

    assert_eq!(&actual, expected);
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

/// Checks one empty field without changing any other field in the valid fixture.
#[track_caller]
fn assert_empty_field_error<T>(encoded: &serde_json::Value, pointer: &str, expected: &str)
where
    T: DeserializeOwned + Debug,
{
    let mut invalid = encoded.clone();
    *invalid
        .pointer_mut(pointer)
        .expect("the case names a field in the serialized fixture") =
        serde_json::Value::String(String::new());

    let error = serde_json::from_value::<T>(invalid)
        .expect_err("the fixture with the empty required field must be rejected");

    assert!(
        error.to_string().contains(expected),
        "expected an error containing {expected:?} at {pointer}, got {error}"
    );
}

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

fn capture_record() -> CaptureRecord {
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
            "doc": true,
            "doctest": true,
            "test": true
        },
        "profile": {
            "opt_level": "s",
            "debuginfo": 1,
            "debug_assertions": false,
            "overflow_checks": true,
            "test": false
        },
        "features": ["alpha", "beta"],
        "filenames": [
            "/workspace/target/libexample.rlib",
            "/workspace/target/libexample.rmeta"
        ],
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

fn instance_record() -> InstanceRecord {
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

fn instance_manifest() -> InstanceManifest {
    InstanceManifest::new(
        capture_id(),
        vec![instance_record()],
        vec![],
        llvm_provenance(),
        LlvmCollection::NotCaptured(UnsupportedLlvmConfiguration::UnverifiedCompiler),
    )
    .expect("the fixture instance manifest is valid")
}

fn llvm_provenance() -> LlvmProvenance {
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

#[test]
fn rejects_noncanonical_capture_ids() {
    let encoded =
        serde_json::to_string(&capture_record()).expect("the fixture record can be encoded");
    let noncanonical = encoded.replace(
        "zyxwvutsrqponmlkzyxwvutsrqponmlk",
        "cap_0123456789abcdef0123456789abcdef",
    );

    assert_record_error(&noncanonical, "capture ID must contain exactly 32");
}

#[test]
fn round_trips_a_valid_record() {
    let expected = capture_record();
    let encoded = serde_json::to_vec(&expected).expect("the fixture record can be encoded");
    let actual = serde_json::from_slice::<CaptureRecord>(&encoded)
        .expect("the encoded fixture record can be read");

    assert_eq!(actual, expected);
}

#[test]
fn rejects_an_unknown_capture_format() {
    let encoded =
        serde_json::to_string(&capture_record()).expect("the fixture record can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":4"#, r#""format_version":5"#);

    assert_record_error(
        &unsupported_version,
        "capture format version must be 4, got 5",
    );
}

#[test]
fn reports_the_previous_capture_format_before_its_missing_fields() {
    let mut previous =
        serde_json::to_value(capture_record()).expect("the fixture record can be encoded");
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
    let expected = instance_manifest();
    let encoded = serde_json::to_string(&expected).expect("the fixture manifest can be encoded");
    let actual = serde_json::from_str::<InstanceManifest>(&encoded)
        .expect("the encoded fixture manifest can be read");

    assert_eq!(actual, expected);
    assert_eq!(actual.format_version(), 4);
    assert_eq!(actual.capture_id(), &capture_id());
    assert!(!encoded.contains("body"));
}

#[test]
fn rejects_an_unknown_instance_manifest_format() {
    let encoded =
        serde_json::to_string(&instance_manifest()).expect("the fixture manifest can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":4"#, r#""format_version":5"#);
    let error = serde_json::from_str::<InstanceManifest>(&unsupported_version)
        .expect_err("the unsupported manifest must be rejected");

    assert_eq!(error.to_string(), "capture format version must be 4, got 5");
}

#[test]
fn rejects_a_malformed_instance_manifest() {
    let encoded =
        serde_json::to_string(&instance_manifest()).expect("the fixture manifest can be encoded");
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
    let duplicate = instance_record();

    let error = InstanceManifest::new(
        capture_id(),
        vec![duplicate.clone(), duplicate],
        vec![],
        llvm_provenance(),
        LlvmCollection::Collected(vec![]),
    )
    .expect_err("duplicate instance identities must be rejected");

    assert!(error.to_string().contains("duplicate instance"));
}

#[test]
fn rejects_each_invalid_compiler_path() {
    let root = std::env::current_dir().expect("the test invocation directory is available");

    let cases = [
        (PathBuf::new(), "rustc path"), // Reject an empty rustc path.
        (PathBuf::from("rustc"), "relative path"), // Reject a relative rustc path.
        (root.join("toolchain/./rustc"), "not lexically normalized"), // Reject `.` components.
        (root.join("toolchain/../rustc"), "not lexically normalized"), // Reject `..` components.
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
        PathBuf::new(),                    // Reject an empty sysroot path.
        PathBuf::from("toolchain"),        // Reject a relative sysroot path.
        root.join("toolchain/../sysroot"), // Reject `..` components.
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
    let encoded =
        serde_json::to_value(capture_record()).expect("the fixture record can be encoded");

    let cases = [
        ("/compiler/release", "rustc release"), // Require the release string.
        ("/compiler/commit_hash", "rustc commit hash"), // Require the commit hash.
        ("/compiler/host", "rustc host"),       // Require the host triple.
    ];

    for (pointer, field) in cases {
        assert_empty_field_error::<CaptureRecord>(&encoded, pointer, field);
    }
}

#[test]
fn deserialization_rejects_each_empty_instance_text_field() {
    let encoded =
        serde_json::to_value(instance_manifest()).expect("the fixture manifest can be encoded");

    let cases = [
        (
            "/instances/0/definition/crate_name",
            "definition crate name",
        ), // Require the definition crate name.
        ("/instances/0/definition/definition_path", "definition path"), // Require the path.
        ("/instances/0/display_name", "instance display name"),         // Require the display name.
        ("/instances/0/raw_symbol", "instance raw symbol"),             // Require the raw symbol.
        ("/instances/0/placements/0/codegen_unit", "codegen unit"),     // Require the codegen unit.
        ("/instances/0/placements/0/linkage", "placement linkage"),     // Require the linkage.
        (
            "/instances/0/placements/0/visibility",
            "placement visibility",
        ), // Require the visibility.
    ];

    for (pointer, field) in cases {
        assert_empty_field_error::<InstanceManifest>(&encoded, pointer, field);
    }
}

#[test]
fn deserialization_rejects_unknown_evidence_fields() {
    let encoded =
        serde_json::to_value(instance_manifest()).expect("the fixture manifest can be encoded");

    let pointers = [
        "",                          // Reject unknown manifest fields.
        "/instances/0",              // Reject unknown instance fields.
        "/instances/0/definition",   // Reject unknown definition fields.
        "/instances/0/placements/0", // Reject unknown placement fields.
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

    let mut invalid =
        serde_json::to_value(capture_record()).expect("the fixture record can be encoded");
    invalid["compiler"]["unknown"] = serde_json::Value::Bool(true);
    let error = serde_json::from_value::<CaptureRecord>(invalid)
        .expect_err("the unknown compiler field must be rejected");

    assert!(error.to_string().contains("unknown field `unknown`"));
}

#[test]
fn rejects_an_unknown_target_kind() {
    let encoded =
        serde_json::to_string(&capture_record()).expect("the fixture record can be encoded");
    let invalid_target = encoded.replace(r#""kind":"lib""#, r#""kind":"test""#);

    assert_record_error(&invalid_target, "unknown variant `test`");
}

#[test]
fn rejects_each_empty_text_field() {
    let encoded =
        serde_json::to_value(capture_record()).expect("the fixture record can be encoded");

    let cases = [
        ("/build/package", "package name"), // Require the package name.
        ("/build/package_version", "package version"), // Require the package version.
        ("/build/target/name", "target name"), // Require the target name.
        ("/build/profile", "profile"),      // Require the profile name.
        ("/build/cargo_program", "Cargo program"), // Require the Cargo program.
    ];

    for (pointer, field) in cases {
        let expected = format!("{field} must contain a valid value");

        assert_empty_field_error::<CaptureRecord>(&encoded, pointer, &expected);
    }
}

#[test]
fn rejects_an_empty_cargo_argument_list() {
    let encoded =
        serde_json::to_string(&capture_record()).expect("the fixture record can be encoded");
    let empty_arguments =
        encoded.replace(r#""cargo_arguments":["rustc"]"#, r#""cargo_arguments":[]"#);

    assert_record_error(
        &empty_arguments,
        "Cargo arguments must contain a valid value",
    );
}

#[test]
fn rejects_each_invalid_invocation_directory() {
    let encoded =
        serde_json::to_string(&capture_record()).expect("the fixture record can be encoded");

    let cases = [
        ("", "an empty path"),    // Reject an empty path.
        (".", "a relative path"), // Reject a relative path.
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
    let encoded =
        serde_json::to_string(&capture_record()).expect("the fixture record can be encoded");
    let unknown_field = encoded.replacen('{', r#"{"unknown":true,"#, 1);

    assert_record_error(&unknown_field, "unknown field `unknown`");
}

#[test]
fn validates_request_keys_at_parse_and_deserialization() {
    let valid = analysis().request_key().to_string();

    assert_eq!(valid.parse::<CaptureKey>().unwrap().as_str(), valid);

    for invalid in [
        String::new(),  // Reject an empty digest.
        "a".repeat(63), // Reject a short digest.
        "a".repeat(65), // Reject a long digest.
        "A".repeat(64), // Reject an uppercase digest.
        "g".repeat(64), // Reject a non-hexadecimal digest.
        "é".repeat(32), // Reject a non-ASCII digest.
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
    assert_json_round_trip(&first);

    for invalid in [
        "",                                     // Reject an empty token.
        "0123456789ab4def8123456789abcde",      // Reject a short token.
        "01234567-89ab-4def-8123-456789abcdef", // Reject a hyphenated UUID.
        "0123456789AB4DEF8123456789ABCDEF",     // Reject an uppercase UUID.
        "0123456789ab1def8123456789abcdef",     // Reject another UUID version.
        "0123456789ab4def7123456789abcdef",     // Reject another UUID variant.
        "0123456789ab4def8123456789abcdeg",     // Reject a non-hexadecimal digit.
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
        ("/package_id", serde_json::json!("")), // Require the package identity.
        ("/target/name", serde_json::json!("")), // Require the target name.
        ("/target/kind", serde_json::json!([])), // Require the target kinds.
        ("/target/crate_types", serde_json::json!([])), // Require the crate types.
        ("/target/kind", serde_json::json!([""])), // Reject an empty target kind.
        ("/target/crate_types", serde_json::json!([""])), // Reject an empty crate type.
        ("/profile/opt_level", serde_json::json!("")), // Require the optimization level.
        ("/manifest_path", serde_json::json!("Cargo.toml")), // Reject a relative manifest path.
        ("/target/src_path", serde_json::json!("src/lib.rs")), // Reject a relative source path.
        (
            "/target/src_path",
            serde_json::json!("/workspace/../lib.rs"),
        ), // Reject `..` components.
        ("/filenames", serde_json::json!(["target/lib.rlib"])), // Reject a relative output path.
        ("/executable", serde_json::json!("target/example")), // Reject a relative executable path.
        ("/features", serde_json::json!([""])), // Reject an empty feature.
        ("/target/required-features", serde_json::json!([""])), // Reject an empty feature.
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
    let mut encoded = serde_json::to_value(capture_record()).unwrap();
    encoded.as_object_mut().unwrap().remove("analysis");

    assert_record_error(
        &encoded.to_string(),
        "analysis must contain a valid value, got no value",
    );
}
