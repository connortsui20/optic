use std::fs;
use std::path::PathBuf;

use optic_records::BuildRecord;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::CargoTargetKind;
use optic_records::RustcInvocation;
use optic_records::TargetRecord;
use optic_records::ToolchainRecord;

use crate::RECORD_FILE_NAME;
use crate::Store;

fn record(id: &str, completed_at_unix_ms: u64) -> CaptureRecord {
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
        id.parse().expect("the fixture capture ID is valid"),
        completed_at_unix_ms,
        build,
        toolchain,
    )
}

#[test]
fn publishes_immutable_records_and_lists_newest_first() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::open(temporary.path());
    let older = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", 1_000);
    let newer = record("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx", 2_000);

    store
        .publish(&older)
        .expect("the older capture can be published");
    store
        .publish(&newer)
        .expect("the newer capture can be published");

    assert!(store.publish(&older).is_err());
    assert_eq!(
        store.captures().expect("captures can be listed"),
        vec![newer, older]
    );
}

#[test]
fn ignores_staging_and_rejects_invalid_completed_data() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::open(temporary.path());
    let staged = store.root.join("staging/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy");
    fs::create_dir_all(&staged).expect("the staging directory can be created");
    fs::write(staged.join(RECORD_FILE_NAME), b"not complete")
        .expect("the staging record can be written");

    assert!(store.captures().expect("staging is not visible").is_empty());

    let completed_id: CaptureId = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx"
        .parse()
        .expect("the completed directory ID is valid");
    let completed = store.root.join("captures").join(completed_id.as_str());
    fs::create_dir_all(&completed).expect("the completed directory can be created");
    fs::write(completed.join(RECORD_FILE_NAME), b"not JSON")
        .expect("the invalid record can be written");

    assert!(store.captures().is_err());
}

#[test]
fn lists_legacy_format_version_one_records_without_rewriting_them() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let store = Store::open(temporary.path());
    let expected = record("zyxwvutsrqponmlkzyxwvutsrqponmlk", 1_000);
    let encoded = serde_json::to_string_pretty(&expected)
        .expect("the canonical fixture record can be encoded")
        .replace(
            "zyxwvutsrqponmlkzyxwvutsrqponmlk",
            "cap_0123456789abcdef0123456789abcdef",
        );
    let legacy = store
        .root
        .join("captures/cap_0123456789abcdef0123456789abcdef");
    fs::create_dir_all(&legacy).expect("the legacy capture directory can be created");
    fs::write(legacy.join(RECORD_FILE_NAME), encoded)
        .expect("the legacy capture record can be written");

    assert_eq!(
        store.captures().expect("the legacy capture can be listed"),
        vec![expected]
    );
    assert!(legacy.is_dir());
}
