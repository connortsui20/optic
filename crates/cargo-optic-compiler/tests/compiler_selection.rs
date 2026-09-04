//! Exercises compiler replacement through real Cargo and retained wrapper processes.
//!
//! Child test processes isolate compiler environment variables from concurrent tests.

#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use optic_compiler::BuildRequest;
use optic_compiler::CargoTarget;
use optic_compiler::collect_build;
use optic_compiler::discover_workspace;

#[track_caller]
fn check_collection(bare_environment: bool, changed_identity: Option<&str>) {
    let temporary = tempfile::tempdir().expect("the fixture directory can be created");
    let directory = temporary.path();
    fs::create_dir(directory.join("src")).unwrap();
    fs::create_dir(directory.join(".cargo")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        "[package]\nname = \"selection_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(directory.join("src/lib.rs"), "pub fn collected() {}\n").unwrap();

    let configuration = if let Some(field) = changed_identity {
        let sysroot = Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap();
        assert!(sysroot.status.success());
        let sysroot = String::from_utf8(sysroot.stdout).unwrap();
        let rustc = Path::new(sysroot.trim()).join("bin/rustc");
        let rustc = rustc.to_str().unwrap().replace('\'', "'\"'\"'");

        // A proxy fixture keeps executable identity fixed while the wrapper changes its reports.
        write_script(
            &directory.join("rustup"),
            &format!(
                "#!/bin/sh\n\
                 if [ \"$OPTIC_TEST_CHANGED_IDENTITY\" = commit ] && [ \"$1\" = -vV ]; then\n\
                   '{rustc}' -vV | sed 's/^commit-hash: .*/commit-hash: changed/'\n\
                 elif [ \"$OPTIC_TEST_CHANGED_IDENTITY\" = sysroot ] && [ \"$1\" = --print ] && [ \"$2\" = sysroot ]; then\n\
                   pwd\n\
                 else\n\
                   exec '{rustc}' \"$@\"\n\
                 fi\n"
            ),
        );
        fs::hard_link(directory.join("rustup"), directory.join("rustc")).unwrap();
        write_script(
            &directory.join("wrapper"),
            &format!("#!/bin/sh\nexport OPTIC_TEST_CHANGED_IDENTITY={field}\nexec \"$@\"\n"),
        );

        "[build]\nrustc = \"./rustc\"\nrustc-wrapper = \"./wrapper\"\n"
    } else {
        "[build]\nrustc = \"rustc\"\n"
    };
    fs::write(directory.join(".cargo/config.toml"), configuration).unwrap();

    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--exact", "collect_in_child", "--nocapture"])
        .env("OPTIC_TEST_WORKSPACE", directory)
        .env_remove("OPTIC_TEST_CHANGED_IDENTITY")
        .env_remove("RUSTC")
        .env_remove("CARGO_BUILD_RUSTC")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER");
    if bare_environment {
        fs::write(directory.join(".cargo/config.toml"), "").unwrap();
        command.env("RUSTC", "rustc");
    }
    let output = command.output().expect("the collector test child can run");
    let diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    if let Some(field) = changed_identity {
        assert!(!output.status.success(), "{diagnostics}");
        assert!(
            diagnostics.contains(&format!(
                "selected rustc {field} must match the prepared compiler"
            )),
            "{diagnostics}",
        );
    } else {
        assert!(output.status.success(), "{diagnostics}");
    }
}

fn write_script(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn collects_with_a_bare_configured_compiler() {
    check_collection(false, None);
}

#[test]
fn collects_with_a_bare_environment_compiler() {
    check_collection(true, None);
}

#[test]
fn rejects_a_wrapper_changed_compiler_commit() {
    check_collection(false, Some("commit"));
}

#[test]
fn rejects_a_wrapper_changed_compiler_sysroot() {
    check_collection(false, Some("sysroot"));
}

#[test]
fn collect_in_child() {
    let Some(directory) = env::var_os("OPTIC_TEST_WORKSPACE") else {
        return;
    };
    let workspace = discover_workspace(Path::new(&directory)).unwrap();
    let request = BuildRequest::new("selection_fixture", CargoTarget::Library, "release").unwrap();
    collect_build(&workspace, &request).expect("the compiler must collect successfully");
}
