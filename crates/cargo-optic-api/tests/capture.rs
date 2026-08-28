use std::fs;
use std::path::Path;

use optic::BuildRequest;
use optic::CaptureRecord;
use optic::CargoTarget;
use optic::CargoTargetKind;
use optic::Error;
use optic::Optic;
use tempfile::TempDir;

fn fixture_workspace(version: &str) -> TempDir {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    write_manifest(&temporary, version);
    fs::create_dir(temporary.path().join("src"))
        .expect("the fixture source directory can be created");
    fs::write(
        temporary.path().join("src/lib.rs"),
        "pub fn captured() -> bool { true }\n",
    )
    .expect("the fixture source can be written");

    temporary
}

fn write_manifest(workspace: &TempDir, version: &str) {
    fs::write(
        workspace.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"capture_fixture\"\nversion = \"{version}\"\nedition = \"2024\"\n"
        ),
    )
    .expect("the fixture manifest can be written");
}

fn library_request() -> BuildRequest {
    BuildRequest::new("capture_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid")
}

#[track_caller]
fn assert_library_capture(capture: &CaptureRecord, workspace: &Path) {
    let build = capture.build();

    assert_eq!(build.package(), "capture_fixture");
    assert_eq!(build.package_version(), "0.1.0");
    assert_eq!(build.target().name(), "capture_fixture");
    assert_eq!(build.target().kind(), CargoTargetKind::Lib);
    assert_eq!(build.profile(), "release");
    assert_eq!(build.invocation_directory(), workspace);
}

#[test]
fn rejects_relative_invocation_directories() {
    let error = Optic::open(Path::new("."))
        .err()
        .expect("the relative invocation directory must be rejected");

    assert!(matches!(error, Error::Compiler { .. }));
}

#[test]
fn captures_and_lists_builds_through_the_product_api() {
    let temporary = fixture_workspace("0.1.0");
    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    let request = library_request();
    let first = optic.capture(&request).expect("the first capture succeeds");
    let second = optic
        .capture(&request)
        .expect("the second capture succeeds");

    assert_ne!(first.id(), second.id());

    let captures = optic.captures().expect("completed captures can be listed");
    assert_eq!(captures.len(), 2);
    for capture in &captures {
        assert_library_capture(capture, temporary.path());
    }
}

#[test]
fn failed_target_resolution_does_not_publish_a_capture() {
    let temporary = fixture_workspace("0.1.0");
    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    let request = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("missing".to_owned()),
        "release",
    )
    .expect("the missing target is syntactically valid");

    let error = optic
        .capture(&request)
        .expect_err("the missing target must fail capture");

    assert!(matches!(&error, Error::Compiler { .. }));
    assert!(error.to_string().contains("binary missing"));
    assert!(
        optic
            .captures()
            .expect("completed captures can be listed")
            .is_empty()
    );
}

#[test]
fn failed_cargo_process_does_not_publish_a_capture() {
    let temporary = fixture_workspace("0.1.0");
    fs::write(temporary.path().join("src/lib.rs"), "pub fn broken( {\n")
        .expect("the invalid fixture source can be written");
    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");

    let error = optic
        .capture(&library_request())
        .expect_err("the invalid source must fail capture");

    assert!(matches!(&error, Error::Compiler { .. }));
    assert!(
        optic
            .captures()
            .expect("completed captures can be listed")
            .is_empty()
    );
}

#[test]
fn refreshes_cargo_metadata_for_each_capture() {
    let temporary = fixture_workspace("0.1.0");
    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    write_manifest(&temporary, "0.2.0");

    let capture = optic
        .capture(&library_request())
        .expect("the changed workspace can be captured");

    assert_eq!(capture.build().package_version(), "0.2.0");
}
