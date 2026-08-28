//! Protects Cargo wrapper configuration across capture execution.
//!
//! Real wrapper processes verify behavior that unit tests cannot establish.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use optic::BuildRequest;
use optic::CargoTarget;
use optic::Optic;

fn write_package(directory: &Path, name: &str) {
    fs::create_dir(directory.join("src")).expect("the fixture source directory can be created");
    fs::write(
        directory.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .expect("the fixture manifest can be written");
    fs::write(directory.join("src/lib.rs"), "pub fn wrapped() {}\n")
        .expect("the fixture source can be written");
}

fn compile_logging_wrapper(path: &Path) -> PathBuf {
    let source = path.with_extension("rs");
    fs::write(
        &source,
        r#"
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;
use std::process::exit;

fn main() {
    let executable = env::current_exe().expect("the wrapper path is available");
    let marker = executable.with_extension("marker");
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(marker)
            .expect("the marker can be opened"),
        "invoked",
    )
    .expect("the marker can be written");

    let mut arguments = env::args_os().skip(1);
    let program = arguments.next().expect("the wrapped program is present");
    let status = Command::new(program)
        .args(arguments)
        .status()
        .expect("the wrapped program can run");

    exit(status.code().unwrap_or(1));
}
"#,
    )
    .expect("the wrapper source can be written");
    let status = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(path)
        .status()
        .expect("rustc can compile the wrapper fixture");
    assert!(
        status.success(),
        "rustc failed to compile the wrapper fixture"
    );

    path.with_extension("marker")
}

#[test]
fn preserves_the_configured_wrapper_chain() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let tools = temporary.path().join("tools");
    fs::create_dir_all(temporary.path().join(".cargo"))
        .expect("the Cargo configuration directory can be created");
    fs::create_dir(&tools).expect("the wrapper directory can be created");
    write_package(temporary.path(), "wrapped_fixture");

    let outer_wrapper = tools.join(format!("outer_wrapper{}", std::env::consts::EXE_SUFFIX));
    let workspace_wrapper =
        tools.join(format!("workspace_wrapper{}", std::env::consts::EXE_SUFFIX));
    let outer_marker = compile_logging_wrapper(&outer_wrapper);
    let workspace_marker = compile_logging_wrapper(&workspace_wrapper);
    let outer_name = outer_wrapper
        .file_name()
        .expect("the outer wrapper has a file name")
        .to_string_lossy();
    let workspace_name = workspace_wrapper
        .file_name()
        .expect("the workspace wrapper has a file name")
        .to_string_lossy();
    fs::write(
        temporary.path().join(".cargo/config.toml"),
        format!(
            "[build]\n\
             rustc-wrapper = \"tools/{outer_name}\"\n\
             rustc-workspace-wrapper = \"tools/{workspace_name}\"\n",
        ),
    )
    .expect("the Cargo configuration can be written");

    let optic = Optic::open(temporary.path()).expect("the fixture workspace can be opened");
    let request = BuildRequest::new("wrapped_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid");
    let capture = optic
        .capture(&request)
        .expect("the wrapped Cargo invocation can be captured");

    assert!(outer_marker.is_file());
    assert!(workspace_marker.is_file());
    assert_eq!(capture.build().invocation_directory(), temporary.path());
}

#[test]
fn preserves_configuration_from_the_invocation_directory() {
    let temporary = tempfile::tempdir().expect("the test workspace can be created");
    let member = temporary.path().join("member");
    let wrapper = member.join(format!("tools/wrapper{}", std::env::consts::EXE_SUFFIX));
    fs::create_dir_all(member.join(".cargo"))
        .expect("the member Cargo configuration directory can be created");
    fs::create_dir(member.join("tools")).expect("the wrapper directory can be created");
    fs::write(
        temporary.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
    )
    .expect("the workspace manifest can be written");
    write_package(&member, "member_fixture");
    let marker = compile_logging_wrapper(&wrapper);
    let wrapper_name = wrapper
        .file_name()
        .expect("the wrapper has a file name")
        .to_string_lossy();
    fs::write(
        member.join(".cargo/config.toml"),
        format!("[build]\nrustc-wrapper = \"tools/{wrapper_name}\"\n"),
    )
    .expect("the member Cargo configuration can be written");

    let optic = Optic::open(&member).expect("the fixture workspace can be opened from its member");
    let request = BuildRequest::new("member_fixture", CargoTarget::Library, "release")
        .expect("the fixture request is valid");
    let capture = optic
        .capture(&request)
        .expect("the member Cargo configuration can be captured");

    assert!(marker.is_file());
    assert_eq!(capture.build().invocation_directory(), member);
}
