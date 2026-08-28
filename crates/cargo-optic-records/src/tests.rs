use std::path::PathBuf;

use crate::BuildRecord;
use crate::CaptureRecord;
use crate::CargoTargetKind;
use crate::TargetRecord;

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

    CaptureRecord::new(
        "zyxwvutsrqponmlkzyxwvutsrqponmlk"
            .parse()
            .expect("the fixture capture ID is valid"),
        1_000,
        build,
    )
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
