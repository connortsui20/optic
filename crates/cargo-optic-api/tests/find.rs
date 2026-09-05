//! Protects concrete-instance search across the public application boundary.
//!
//! These tests use the actual selected compiler because only rustc can establish monomorphization,
//! placement, and raw-symbol behavior. Evidence remains scoped to the capture that produced it.

use std::fs;
use std::path::Path;

use optic::BuildRequest;
use optic::CaptureRecord;
use optic::CargoTarget;
use optic::Optic;

const FIRST_SCOPE: &str = "first_scope_kernel";
const SECOND_SCOPE: &str = "second_scope_kernel";
const FIXTURE_SOURCE: &str = include_str!("fixtures/find/main.rs");

fn write_fixture(workspace: &Path, scoped_kernel: &str) {
    fs::create_dir_all(workspace.join("src")).expect("the fixture source directory can be created");
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"find_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .expect("the fixture manifest can be written");
    fs::write(
        workspace.join("src/main.rs"),
        FIXTURE_SOURCE.replace("SCOPED_KERNEL", scoped_kernel),
    )
    .expect("the fixture source can be written");
}

struct CapturedFindFixture {
    /// Keeps the captured workspace and its store alive for each test.
    workspace: tempfile::TempDir,
    /// The application handle bound to the temporary workspace.
    optic: Optic,
    /// The reusable request for captures before and after a source change.
    request: BuildRequest,
    /// The first completed capture against which searches are scoped.
    first: CaptureRecord,
}

impl CapturedFindFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("the test workspace can be created");
        write_fixture(workspace.path(), FIRST_SCOPE);
        let optic = Optic::open(workspace.path()).expect("the fixture workspace can be opened");
        let request = BuildRequest::new(
            "find_fixture",
            CargoTarget::Binary("find_fixture".to_owned()),
            "release",
        )
        .expect("the fixture request is valid");
        let first = optic
            .capture(&request)
            .expect("the first generic fixture capture succeeds");

        Self {
            workspace,
            optic,
            request,
            first,
        }
    }
}

#[test]
fn finds_concrete_generic_instances() {
    let fixture = CapturedFindFixture::new();
    let found = fixture
        .optic
        .find(fixture.first.id(), "find_fixture::generic_kernel", 100)
        .expect("the generic definition can be found");

    assert_eq!(found.capture_id(), fixture.first.id());
    assert_eq!(found.instances().len(), 2);
    assert_ne!(
        found.instances()[0].display_name(),
        found.instances()[1].display_name(),
    );
    assert_ne!(
        found.instances()[0].raw_symbol(),
        found.instances()[1].raw_symbol(),
    );
}

#[test]
fn finds_nested_generics_and_canonical_trait_methods() {
    let fixture = CapturedFindFixture::new();
    let nested = fixture
        .optic
        .find(fixture.first.id(), "nested_kernel::chunk", 100)
        .expect("a nested generic instance remains searchable");
    assert!(!nested.instances().is_empty());
    assert_eq!(nested.capture_id(), fixture.first.id());

    let trait_method = fixture
        .optic
        .find(
            fixture.first.id(),
            "<find_fixture::LocalKernel as find_fixture::Kernel>::trait_kernel",
            100,
        )
        .expect("the fully qualified trait method can be found");
    assert_eq!(trait_method.instances().len(), 1);
}

#[test]
fn isolates_instances_between_captures() {
    let fixture = CapturedFindFixture::new();
    write_fixture(fixture.workspace.path(), SECOND_SCOPE);
    let second = fixture
        .optic
        .capture(&fixture.request)
        .expect("the changed generic fixture capture succeeds");

    assert!(
        !fixture
            .optic
            .find(fixture.first.id(), FIRST_SCOPE, 100)
            .expect("the first capture remains searchable")
            .instances()
            .is_empty()
    );
    assert!(
        fixture
            .optic
            .find(fixture.first.id(), SECOND_SCOPE, 100)
            .expect("the first capture remains isolated")
            .instances()
            .is_empty()
    );
    assert!(
        fixture
            .optic
            .find(second.id(), FIRST_SCOPE, 100)
            .expect("the second capture remains isolated")
            .instances()
            .is_empty()
    );
    assert!(
        !fixture
            .optic
            .find(second.id(), SECOND_SCOPE, 100)
            .expect("the second capture is searchable")
            .instances()
            .is_empty()
    );
}
