use std::fs;

use optic::BuildRequest;
use optic::CargoTarget;
use optic::Optic;
use tempfile::TempDir;

fn workspace(version: &str) -> TempDir {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    write_manifest(&temporary, version);
    fs::create_dir(temporary.path().join("src")).expect("the fixture source directory can exist");
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

fn request() -> BuildRequest {
    BuildRequest::new("capture_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid")
}

#[test]
fn captures_and_lists_builds_through_the_product_api() {
    let temporary = workspace("0.1.0");
    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    let request = request();
    let first = optic.capture(&request).expect("the first capture succeeds");
    let second = optic
        .capture(&request)
        .expect("the second capture succeeds");
    let invalid = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("missing".to_owned()),
        "release",
    )
    .expect("the missing target is syntactically valid");

    assert_ne!(first.id(), second.id());
    assert!(optic.capture(&invalid).is_err());

    let captures = optic.captures().expect("completed captures can be listed");
    assert_eq!(captures.len(), 2);
    assert!(captures.iter().all(|capture| {
        capture.build().package() == "capture_fixture"
            && capture.build().target().kind() == optic::CargoTargetKind::Lib
            && capture.build().profile() == "release"
            && !capture.toolchain().commit_hash().is_empty()
    }));
}

#[test]
fn refreshes_cargo_metadata_for_each_capture() {
    let temporary = workspace("0.1.0");
    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    write_manifest(&temporary, "0.2.0");

    let capture = optic
        .capture(&request())
        .expect("the changed workspace can be captured");

    assert_eq!(capture.build().package_version(), "0.2.0");
}
