//! Protects durable record validation.
//!
//! Constructors and deserializers must reject the same malformed fields.

use std::path::PathBuf;

use crate::BuildRecord;
use crate::CaptureId;
use crate::CaptureRecord;
use crate::CargoTargetKind;
use crate::CompilerIdentity;
use crate::DefinitionRecord;
use crate::InstanceManifest;
use crate::InstanceRecord;
use crate::PlacementRecord;
use crate::TargetRecord;

fn capture_id() -> CaptureId {
    "zyxwvutsrqponmlkzyxwvutsrqponmlk"
        .parse()
        .expect("the fixture capture ID is valid")
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

    CaptureRecord::new(capture_id(), 1_000, build)
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
    )
    .expect("the fixture instance is valid")
}

fn manifest() -> InstanceManifest {
    InstanceManifest::new(capture_id(), vec![instance()])
        .expect("the fixture instance manifest is valid")
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
fn rejects_an_unknown_format_version() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":1"#, r#""format_version":2"#);

    assert_record_error(
        &unsupported_version,
        "capture format version must be 1, got 2",
    );
}

#[test]
fn round_trips_a_current_instance_manifest_without_body_metadata() {
    let expected = manifest();
    let encoded = serde_json::to_string(&expected).expect("the fixture manifest can be encoded");
    let actual = serde_json::from_str::<InstanceManifest>(&encoded)
        .expect("the encoded fixture manifest can be read");

    assert_eq!(actual, expected);
    assert_eq!(actual.format_version(), 1);
    assert_eq!(actual.capture_id(), record().id());
    assert!(!encoded.contains("body"));
}

#[test]
fn rejects_a_wrong_instance_manifest_version() {
    let encoded = serde_json::to_string(&manifest()).expect("the fixture manifest can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":1"#, r#""format_version":2"#);
    let error = serde_json::from_str::<InstanceManifest>(&unsupported_version)
        .expect_err("the unsupported manifest must be rejected");

    assert_eq!(error.to_string(), "capture format version must be 1, got 2");
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
    )
    .expect_err("duplicate codegen units must be rejected");

    assert!(error.to_string().contains("duplicate codegen unit"));
}

#[test]
fn rejects_duplicate_instance_identities() {
    let duplicate = instance();
    let error = InstanceManifest::new(capture_id(), vec![duplicate.clone(), duplicate])
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
