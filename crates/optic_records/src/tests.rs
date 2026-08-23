use std::path::PathBuf;

use crate::BuildRecord;
use crate::CaptureRecord;
use crate::CargoTargetKind;
use crate::RustcInvocation;
use crate::TargetRecord;
use crate::ToolchainRecord;

fn record() -> CaptureRecord {
    let target =
        TargetRecord::new("example", CargoTargetKind::Lib).expect("the fixture target is valid");
    let build = BuildRecord::new(
        "example",
        "0.1.0",
        target,
        "release",
        PathBuf::from("cargo"),
        vec!["rustc".to_owned()],
    )
    .expect("the fixture build is valid");
    let invocation = RustcInvocation::new(PathBuf::from("rustc"), None, None)
        .expect("the fixture compiler invocation is valid");
    let toolchain = ToolchainRecord::new(
        invocation,
        "1.98.0",
        "0123456789abcdef",
        "test-host",
        "22.1.8",
    )
    .expect("the fixture toolchain is valid");

    CaptureRecord::new(
        "cap_0123456789abcdef0123456789abcdef"
            .parse()
            .expect("the fixture capture ID is valid"),
        1_000,
        build,
        toolchain,
    )
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
fn rejects_invalid_records_during_deserialization() {
    let encoded = serde_json::to_string(&record()).expect("the fixture record can be encoded");
    let unsupported_version = encoded.replace(r#""format_version":1"#, r#""format_version":2"#);
    let invalid_target = encoded.replace(r#""kind":"lib""#, r#""kind":"test""#);
    let empty_package = encoded.replace(r#""package":"example""#, r#""package":"""#);

    assert!(serde_json::from_str::<CaptureRecord>(&unsupported_version).is_err());
    assert!(serde_json::from_str::<CaptureRecord>(&invalid_target).is_err());
    assert!(serde_json::from_str::<CaptureRecord>(&empty_package).is_err());
}
