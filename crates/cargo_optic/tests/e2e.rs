use std::ffi::OsStr;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::process::Output;

#[test]
fn captures_a_fixture_target_and_lists_the_completed_record() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());

    let captured = run(
        temporary.path(),
        ["capture", "-p", "capture_fixture", "--release", "lib"],
    );
    assert_success(&captured);
    let captured_text = String::from_utf8(captured.stdout).expect("capture output is UTF-8");
    let capture_id = captured_text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Captured "))
        .expect("capture output starts with its opaque ID");

    assert!(capture_id.starts_with("cap_"));
    assert!(captured_text.contains("Package    capture_fixture 0.1.0"));
    assert!(captured_text.contains("Target     lib capture_fixture"));
    assert!(captured_text.contains("Profile    release"));
    assert!(captured_text.contains("Commit     "));
    assert!(
        temporary
            .path()
            .join("target/release/libcapture_fixture.rlib")
            .is_file()
    );

    let listed = run(temporary.path(), ["captures"]);
    assert_success(&listed);
    let listed_text = String::from_utf8(listed.stdout).expect("listing output is UTF-8");

    assert!(listed_text.starts_with("Captures\n\n"));
    assert!(listed_text.contains(&format!("Capture {capture_id}")));
    assert!(listed_text.contains("Package    capture_fixture 0.1.0"));
    assert!(listed_text.contains("Target     lib capture_fixture"));
}

#[cfg(unix)]
#[test]
fn honors_cargo_wrapper_environment_overrides() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    copy_fixture(temporary.path());
    fs::create_dir(temporary.path().join(".cargo"))
        .expect("the Cargo configuration directory can be created");
    fs::write(
        temporary.path().join(".cargo/config.toml"),
        "[build]\nrustc-wrapper = \"missing_outer\"\nrustc-workspace-wrapper = \"missing_workspace\"\n",
    )
    .expect("the Cargo configuration can be written");
    let wrapper = temporary.path().join("wrapper");
    fs::write(&wrapper, "#!/bin/sh\nexec \"$@\"\n").expect("the wrapper fixture can be written");
    let mut permissions = fs::metadata(&wrapper)
        .expect("the wrapper fixture metadata can be read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper, permissions).expect("the wrapper fixture can be executable");

    let mut overridden = command(temporary.path());
    let overridden = overridden
        .env("CARGO_BUILD_RUSTC_WRAPPER", "missing_config_outer")
        .env(
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
            "missing_config_workspace",
        )
        .env("RUSTC_WRAPPER", &wrapper)
        .env("RUSTC_WORKSPACE_WRAPPER", &wrapper)
        .args(["capture", "-p", "capture_fixture", "--release", "lib"])
        .output()
        .expect("the Cargo Optic binary can run");
    assert_success(&overridden);

    let mut configured = command(temporary.path());
    let configured = configured
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env("CARGO_BUILD_RUSTC_WRAPPER", &wrapper)
        .env("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", &wrapper)
        .args(["capture", "-p", "capture_fixture", "--release", "lib"])
        .output()
        .expect("the Cargo Optic binary can run");
    assert_success(&configured);

    let mut disabled = command(temporary.path());
    let disabled = disabled
        .env("CARGO_BUILD_RUSTC_WRAPPER", "missing_config_outer")
        .env(
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
            "missing_config_workspace",
        )
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .args(["capture", "-p", "capture_fixture", "--release", "lib"])
        .output()
        .expect("the Cargo Optic binary can run");
    assert_success(&disabled);
}

fn run<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command(directory)
        .args(arguments)
        .output()
        .expect("the Cargo Optic binary can run")
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo_optic"));
    command.current_dir(directory);

    command
}

fn copy_fixture(directory: &Path) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/capture");
    fs::copy(fixture.join("Cargo.toml"), directory.join("Cargo.toml"))
        .expect("the fixture manifest can be copied");
    fs::create_dir(directory.join("src")).expect("the fixture source directory can be created");
    fs::copy(fixture.join("src/lib.rs"), directory.join("src/lib.rs"))
        .expect("the fixture source can be copied");
}

#[track_caller]
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
