//! Protects capture behavior across the public application boundary.
//!
//! Tests run the real compiler and store through [`Optic`](optic::Optic); subsystem tests cover
//! local errors.

mod common;

use std::fs;
use std::path::Path;

use optic::BuildRequest;
use optic::CaptureError;
use optic::CapturePolicy;
use optic::CaptureRecord;
use optic::CargoTarget;
use optic::CargoTargetKind;
use optic::Error;
use optic::Optic;

use common::workspace_in_child;

#[track_caller]
fn library_request() -> BuildRequest {
    BuildRequest::new("capture_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid")
}

#[track_caller]
fn assert_library_capture(capture: &CaptureRecord, workspace: &Path) {
    let build = capture.build();
    let compiler = capture.compiler();

    assert_eq!(build.package(), "capture_fixture");
    assert_eq!(build.package_version(), "0.1.0");
    assert_eq!(build.target().name(), "capture_fixture");
    assert_eq!(build.target().kind(), CargoTargetKind::Lib);
    assert_eq!(build.profile(), "release");
    assert_eq!(build.invocation_directory(), workspace);

    assert!(compiler.rustc().is_absolute());
    assert!(!compiler.release().is_empty());
    assert!(!compiler.commit_hash().is_empty());
    assert!(!compiler.host().is_empty());
    assert!(compiler.sysroot().is_absolute());
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
    let Some(workspace) = workspace_in_child(
        "captures_and_lists_builds_through_the_product_api",
        "capture",
    ) else {
        return;
    };

    let optic = Optic::open(&workspace).expect("the fixture workspace can be opened");
    let request = library_request();

    let first = optic
        .capture(&request, CapturePolicy::Reuse)
        .expect("the first capture succeeds")
        .into_record();
    let second = optic
        .capture(&request, CapturePolicy::Reuse)
        .expect("the second capture succeeds")
        .into_record();

    assert_eq!(first.id(), second.id());
    assert_eq!(first.completed_at_unix_ms(), second.completed_at_unix_ms());

    let captures = optic
        .list_captures()
        .expect("completed captures can be listed");

    assert_eq!(captures.len(), 1);

    for capture in &captures {
        assert_library_capture(capture, &workspace);
    }
}

#[test]
fn captures_when_cargo_appends_selected_target_flags() {
    let Some(workspace) = workspace_in_child(
        "captures_when_cargo_appends_selected_target_flags",
        "capture",
    ) else {
        return;
    };

    fs::write(
        workspace.join("build.rs"),
        "fn main() { println!(\"cargo::rustc-cfg=optic_fixture\"); }\n",
    )
    .expect("the fixture build script can be written");

    let optic = Optic::open(&workspace).expect("the fixture workspace can be opened");

    let capture = optic
        .capture(&library_request(), CapturePolicy::Reuse)
        .expect("the selected target can receive build-script flags")
        .into_record();

    assert_eq!(
        capture.build().cargo_arguments(),
        [
            "rustc",
            "--package",
            "capture_fixture",
            "--lib",
            "--profile",
            "release",
        ]
    );
}

#[test]
fn reuses_a_custom_profile_with_its_observed_settings() {
    let Some(workspace) = workspace_in_child(
        "reuses_a_custom_profile_with_its_observed_settings",
        "capture",
    ) else {
        return;
    };

    let optic = Optic::open(&workspace).unwrap();
    let request = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("generic".to_owned()),
        "checked",
    )
    .unwrap();

    let first = optic
        .capture(&request, CapturePolicy::Reuse)
        .unwrap()
        .into_record();
    let second = optic.capture(&request, CapturePolicy::Reuse).unwrap();

    assert!(matches!(second, optic::CaptureOutcome::Reused(_)));
    assert_eq!(first, second.into_record());
    assert_eq!(first.build().profile(), "checked");
    assert_eq!(optic.list_captures().unwrap().len(), 1);
}

#[test]
fn failed_target_resolution_does_not_publish_a_capture() {
    let Some(workspace) = workspace_in_child(
        "failed_target_resolution_does_not_publish_a_capture",
        "capture",
    ) else {
        return;
    };

    let optic = Optic::open(&workspace).expect("the fixture workspace can be opened");
    let request = BuildRequest::new(
        "capture_fixture",
        CargoTarget::Binary("missing".to_owned()),
        "release",
    )
    .expect("the missing target is syntactically valid");

    let error = optic
        .capture(&request, CapturePolicy::Reuse)
        .expect_err("the missing target must fail capture");

    assert!(matches!(
        &error,
        Error::Capture {
            source: CaptureError::Compiler { .. },
            ..
        }
    ));
    assert!(error.to_string().contains("binary missing"));
    assert!(
        optic
            .list_captures()
            .expect("completed captures can be listed")
            .is_empty()
    );
}

#[test]
fn failed_cargo_process_does_not_publish_a_capture() {
    let Some(workspace) =
        workspace_in_child("failed_cargo_process_does_not_publish_a_capture", "capture")
    else {
        return;
    };

    fs::write(workspace.join("src/lib.rs"), "pub fn broken( {\n")
        .expect("the invalid fixture source can be written");

    let optic = Optic::open(&workspace).expect("the fixture workspace can be opened");

    let error = optic
        .capture(&library_request(), CapturePolicy::Reuse)
        .expect_err("the invalid source must fail capture");

    assert!(matches!(
        &error,
        Error::Capture {
            source: CaptureError::Compiler { .. },
            ..
        }
    ));
    assert!(
        optic
            .list_captures()
            .expect("completed captures can be listed")
            .is_empty()
    );

    let unpublished = optic::CaptureId::generate();
    let error = optic
        .find(&unpublished, "broken", 100)
        .expect_err("a failed capture must not leave findable evidence");

    assert!(matches!(error, Error::Evidence { .. }));
}

#[test]
fn refreshes_cargo_metadata_for_each_capture() {
    let Some(workspace) =
        workspace_in_child("refreshes_cargo_metadata_for_each_capture", "capture")
    else {
        return;
    };

    let optic = Optic::open(&workspace).expect("the fixture workspace can be opened");
    let manifest = workspace.join("Cargo.toml");
    let contents = fs::read_to_string(&manifest).unwrap();
    fs::write(
        manifest,
        contents.replace("version = \"0.1.0\"", "version = \"0.2.0\""),
    )
    .unwrap();

    let capture = optic
        .capture(&library_request(), CapturePolicy::Reuse)
        .expect("the changed workspace can be captured")
        .into_record();

    assert_eq!(capture.build().package_version(), "0.2.0");
}
