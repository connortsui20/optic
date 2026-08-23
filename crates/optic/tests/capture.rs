use std::fs;

use optic::BuildRequest;
use optic::CargoTarget;
use optic::Optic;

#[test]
fn captures_and_lists_builds_through_the_product_api() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname = \"capture_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("the fixture manifest can be written");
    fs::create_dir(temporary.path().join("src")).expect("the fixture source directory can exist");
    fs::write(
        temporary.path().join("src/lib.rs"),
        "pub fn captured() -> bool { true }\n",
    )
    .expect("the fixture source can be written");

    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid");
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
