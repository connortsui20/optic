//! Collects one successful selected-target compilation and its concrete instances.
//!
//! [`collect_build`] performs exactly one `cargo rustc` invocation. It returns only after the
//! selected rustc succeeds and the exact-version driver output passes every protocol and durable
//! record check, so callers cannot publish partial compiler evidence.

use std::env;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

use optic_records::BuildRecord;
use optic_records::CompilerIdentity;
use optic_records::InstanceRecord;
use optic_records::TargetRecord;
use snafu::ResultExt;

use crate::BuildRequest;
use crate::Error;
use crate::Workspace;
use crate::build::cargo_arguments;
use crate::build::resolve_package;
use crate::build::resolve_target;
use crate::driver::RustcDriver;
use crate::error::ProcessFailedSnafu;
use crate::error::StartProcessSnafu;
use crate::manifest::read_manifest;
use crate::toolchain::CompilerContext;

/// One successful Cargo build and the compiler evidence collected from its selected target.
pub struct CollectedBuild {
    build: BuildRecord,
    compiler: CompilerIdentity,
    instances: Vec<InstanceRecord>,
}

impl CollectedBuild {
    /// Separates the build provenance, compiler identity, and concrete instances.
    pub fn into_parts(self) -> (BuildRecord, CompilerIdentity, Vec<InstanceRecord>) {
        (self.build, self.compiler, self.instances)
    }
}

/// Runs one explicit Cargo target and collects its concrete compiler instances.
///
/// Cargo inherits the current process's standard input, output, and error streams. Existing
/// transparent global and workspace compiler wrappers retain their order. Probes, dependencies,
/// and unrelated targets pass through without instrumentation. Driver-style wrappers that parse
/// rustc's command line are not supported, and a caching wrapper must run the selected compiler
/// instead of returning a cache hit.
///
/// The compiler command must be its reported sysroot's `rustc` or a standard rustup proxy. Other
/// front-ends are rejected because driver replacement would bypass them. This checks replacement
/// fidelity, not executable authenticity. Cargo Optic removes ambient `RUSTC_BOOTSTRAP` from the
/// Cargo child; its own unstable access remains scoped to the temporary driver build. Every capture
/// deliberately changes Cargo's selected-target fingerprint so a warm capture still runs rustc.
/// Cargo therefore retains one additional fingerprint and artifact set for each capture in the
/// selected target directory.
///
/// # Errors
///
/// Returns an error if the request does not resolve to one workspace target, the selected compiler
/// command is an unsupported front-end, the selected compiler lacks its matching `rustc-dev`
/// component, Cargo or rustc fails, the selected compiler does not run, or the compiler manifest is
/// invalid.
pub fn collect_build(
    workspace: &Workspace,
    request: &BuildRequest,
) -> Result<CollectedBuild, Error> {
    let metadata = workspace.read_metadata(request)?;
    let package = resolve_package(&metadata, request.package())?;
    let target = resolve_target(package, request.target())?;
    let compiler = CompilerContext::discover(workspace)?;
    let temporary = tempfile::Builder::new()
        .prefix("cargo-optic-compiler-")
        .tempdir()
        .map_err(|source| Error::Filesystem {
            operation: "create compiler collection directory below",
            path: env::temp_dir(),
            source,
        })?;
    let manifest_path = temporary.path().join("compiler-manifest.bin");
    let driver = RustcDriver::build(workspace, &compiler, temporary.path())?;
    let selected_target_marker = selected_target_marker(temporary.path())?;

    let cargo_arguments = cargo_arguments(request);
    let mut instrumented_arguments = cargo_arguments.clone();
    instrumented_arguments.push("--".to_owned());
    instrumented_arguments.push(selected_target_marker.clone());

    let mut command = Command::new(workspace.cargo());

    // TODO(connor)[Capture output]: Replace the inherited streams with bounded event supervision
    // when capture reads Cargo JSON. The supervisor must drain both output pipes and stop Cargo
    // when its consumer stops.
    command
        .current_dir(workspace.invocation_directory())
        .args(&instrumented_arguments)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env_remove("RUSTC_BOOTSTRAP");
    driver.configure(
        &mut command,
        &compiler,
        &selected_target_marker,
        &manifest_path,
    )?;

    let status = command.status().with_context(|_| StartProcessSnafu {
        program: workspace.cargo().to_owned(),
    })?;
    if !status.success() {
        return ProcessFailedSnafu {
            program: workspace.cargo().to_owned(),
            status: status.to_string(),
            diagnostics: None,
        }
        .fail();
    }

    if !manifest_path.is_file() {
        return Err(Error::CompilerEnvironment {
            message: format!(
                "selected-target rustc must complete compiler analysis after success, got no manifest at {}",
                manifest_path.display()
            ),
        });
    }

    let output = read_manifest(&manifest_path, compiler.identity())?;
    let target = TargetRecord::new(target.name.clone(), request.target().kind())?;
    let build = BuildRecord::new(
        package.name.to_string(),
        package.version.to_string(),
        target,
        request.profile(),
        workspace.cargo().to_owned(),
        workspace.invocation_directory().to_owned(),
        cargo_arguments,
    )?;

    Ok(CollectedBuild {
        build,
        compiler: output.compiler,
        instances: output.instances,
    })
}

fn selected_target_marker(directory: &Path) -> Result<String, Error> {
    let Some(directory) = directory.to_str() else {
        return Err(Error::CompilerEnvironment {
            message: format!(
                "compiler collection directory must be UTF-8, got {}",
                directory.display()
            ),
        });
    };

    // Cargo passes arguments after `--` only to the final selected-target invocation. The unique
    // value makes Cargo consider every capture stale, which ensures that rustc runs instead of
    // reusing a fresh unit. The outer wrapper removes it before existing wrappers or rustc observe
    // it, but Cargo still retains the resulting fingerprint and build artifact.
    // See <https://doc.rust-lang.org/cargo/commands/cargo-rustc.html#description>.
    Ok(format!("--cfg=cargo_optic_selected_target={directory:?}"))
}
