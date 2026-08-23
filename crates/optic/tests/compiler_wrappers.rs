#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use optic::BuildRequest;
use optic::CargoTarget;
use optic::Optic;

#[test]
fn records_the_compiler_selected_by_the_complete_wrapper_chain() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let tools = temporary.path().join("tools");
    fs::create_dir_all(temporary.path().join(".cargo"))
        .expect("the Cargo configuration directory can be created");
    fs::create_dir(&tools).expect("the wrapper directory can be created");
    fs::create_dir(temporary.path().join("src"))
        .expect("the fixture source directory can be created");
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[package]\nname = \"wrapped_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("the fixture manifest can be written");
    fs::write(temporary.path().join("src/lib.rs"), "pub fn wrapped() {}\n")
        .expect("the fixture source can be written");

    let outer_wrapper = tools.join("outer_wrapper");
    let workspace_wrapper = tools.join("workspace_wrapper");
    let fake_rustc = tools.join("fake_rustc");
    write_executable(&outer_wrapper, "#!/bin/sh\nexec \"$@\"\n");
    write_executable(
        &workspace_wrapper,
        "#!/bin/sh\nshift\nexec \"$(dirname \"$0\")/fake_rustc\" \"$@\"\n",
    );
    write_executable(
        &fake_rustc,
        "#!/bin/sh\n\
if [ \"$1\" = \"-vV\" ]; then\n\
    printf '%s\\n' \\
        'rustc 1.98.0 (fedcba987 2026-08-01)' \\
        'binary: rustc' \\
        'commit-hash: fedcba9876543210' \\
        'commit-date: 2026-08-01' \\
        'host: x86_64-unknown-linux-gnu' \\
        'release: 1.98.0' \\
        'LLVM version: 22.1.8'\n\
    exit 0\n\
fi\n\
exec rustc \"$@\"\n",
    );
    fs::write(
        temporary.path().join(".cargo/config.toml"),
        "[build]\nrustc-wrapper = \"tools/outer_wrapper\"\nrustc-workspace-wrapper = \"tools/workspace_wrapper\"\n",
    )
    .expect("the Cargo configuration can be written");

    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    let request = BuildRequest::new("wrapped_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid");
    let capture = optic
        .capture(&request)
        .expect("the wrapped compiler can be captured");
    let invocation = capture.toolchain().invocation();

    assert_eq!(invocation.rustc_wrapper(), Some(outer_wrapper.as_path()));
    assert_eq!(
        invocation.rustc_workspace_wrapper(),
        Some(workspace_wrapper.as_path())
    );
    assert_eq!(capture.toolchain().commit_hash(), "fedcba9876543210");
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("the wrapper fixture can be written");
    let mut permissions = fs::metadata(path)
        .expect("the wrapper fixture metadata can be read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("the wrapper fixture can be executable");
}
