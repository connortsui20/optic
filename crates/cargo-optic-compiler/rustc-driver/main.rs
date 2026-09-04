//! Collects compiler-owned concrete function instances during one rustc invocation.
//!
//! This file is embedded by the `driver` module of `cargo-optic-compiler` and compiled as a
//! standalone executable during each collection. The selected rustc compiles it against the
//! matching compiler libraries installed by `rustc-dev`.
//!
//! `rustc_private` is Rust's feature name for using implementation crates such as `rustc_driver`,
//! `rustc_hir`, and `rustc_middle`. It does not select a different compiler. These crates are parts
//! of the same rustc toolchain, but their APIs and binary interfaces are private and can change in
//! any release. The executable is therefore compiled with, and verifies at startup, the exact rustc
//! that Cargo selected.
//!
//! The executable has two modes. It normally acts as Cargo's outer rustc wrapper and passes
//! compiler invocations through unchanged. For the selected target, it runs `rustc_driver` with an
//! after-analysis callback. That callback observes the concrete function instances and
//! codegen-unit placements that stable Cargo and rustc interfaces do not expose.
//!
//! The [Rust Compiler Development Guide] describes this custom-driver integration point.
//!
//! [Rust Compiler Development Guide]: https://rustc-dev-guide.rust-lang.org/rustc-driver/intro.html

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

mod arguments;
mod protocol;

use std::collections::HashSet;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;

use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_hir::attrs::Linkage;
use rustc_interface::interface::Compiler;
use rustc_middle::mono::MonoItem;
use rustc_middle::mono::Visibility;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::print::with_resolve_crate_name;

use arguments::remove_selected_target_marker;
use protocol::*;

/// Collects concrete instance placements after rustc completes analysis.
struct InstanceCallbacks {
    /// The incomplete manifest owned until analysis finishes successfully.
    manifest: ManifestWriter,
    /// The completed analysis or write failure retained because the callback cannot return it.
    analysis: Option<io::Result<()>>,
}

/// The actual compiler identity written for validation by the parent process.
struct CompilerIdentity {
    rustc: String,
    release: String,
    commit_hash: String,
    host: String,
    sysroot: String,
}

/// Validated inputs required to run the in-process compiler driver.
struct DriverInvocation {
    arguments: Vec<String>,
    identity: CompilerIdentity,
    manifest_path: PathBuf,
}

/// The identity fields shared by every placement of one concrete function instance.
struct ConcreteInstance {
    definition_crate: String,
    definition_path: String,
    display_name: String,
    raw_symbol: String,
}

/// One rustc codegen-unit placement serialized into the private driver protocol.
struct Placement {
    codegen_unit: String,
    linkage: String,
    visibility: String,
    /// Whether rustc emitted this placement as a codegen-unit-local copy.
    local_copy: bool,
    /// Rustc's estimated instance size before code generation.
    size_estimate: usize,
}

impl Callbacks for InstanceCallbacks {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let partitions = tcx.collect_and_partition_mono_items(());
        let mut instances = HashSet::new();

        for codegen_unit in partitions.codegen_units {
            for (mono_item, data) in codegen_unit.items_in_deterministic_order(tcx) {
                let MonoItem::Fn(instance) = mono_item else {
                    continue;
                };
                if instances.insert(instance) && instances.len() > MAX_INSTANCES {
                    self.analysis = Some(Err(invalid_data(format!(
                        "instance count must not exceed {MAX_INSTANCES}, got {}",
                        instances.len()
                    ))));

                    return Compilation::Stop;
                }

                let definition_id = instance.def_id();
                let definition_crate = tcx.crate_name(definition_id.krate).to_string();
                let definition_path = with_resolve_crate_name!(with_no_trimmed_paths!(
                    tcx.def_path_str(definition_id)
                ));
                let display_name = with_resolve_crate_name!(with_no_trimmed_paths!(
                    tcx.def_path_str_with_args(definition_id, instance.args)
                ));
                let concrete = ConcreteInstance {
                    definition_crate,
                    definition_path,
                    display_name,
                    raw_symbol: tcx.symbol_name(instance).name.to_owned(),
                };
                let placement = Placement {
                    codegen_unit: codegen_unit.name().to_string(),
                    linkage: linkage_name(data.linkage).to_owned(),
                    visibility: visibility_name(data.visibility).to_owned(),
                    local_copy: data.inlined,
                    size_estimate: data.size_estimate,
                };

                if let Err(error) = self.manifest.write_placement(&concrete, &placement) {
                    self.analysis = Some(Err(error));

                    return Compilation::Stop;
                }
            }
        }

        self.analysis = Some(Ok(()));

        Compilation::Continue
    }
}

fn linkage_name(linkage: Linkage) -> &'static str {
    match linkage {
        Linkage::AvailableExternally => "AvailableExternally",
        Linkage::Common => "Common",
        Linkage::ExternalWeak => "ExternalWeak",
        Linkage::External => "External",
        Linkage::Internal => "Internal",
        Linkage::LinkOnceAny => "LinkOnceAny",
        Linkage::LinkOnceODR => "LinkOnceODR",
        Linkage::WeakAny => "WeakAny",
        Linkage::WeakODR => "WeakODR",
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Default => "Default",
        Visibility::Hidden => "Hidden",
        Visibility::Protected => "Protected",
    }
}

fn main() -> ExitCode {
    if env::var_os(WRAPPER_ACTIVE_ENV).is_some() && env::var_os(DRIVER_INNER_ENV).is_none() {
        return run_wrapper();
    }

    run_driver()
}

fn run_driver() -> ExitCode {
    let invocation = match prepare_driver_invocation() {
        Ok(invocation) => invocation,
        Err(error) => return failure(error),
    };

    let manifest = match ManifestWriter::create(&invocation.manifest_path, &invocation.identity) {
        Ok(manifest) => manifest,
        Err(error) => {
            return failure(format!(
                "failed to create {}: {error}",
                invocation.manifest_path.display()
            ));
        }
    };
    let mut callbacks = InstanceCallbacks {
        manifest,
        analysis: None,
    };
    let exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&invocation.arguments, &mut callbacks);
    });
    if exit_code != ExitCode::SUCCESS {
        return exit_code;
    }

    match callbacks.analysis {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            return failure(format!(
                "failed to write {}: {error}",
                invocation.manifest_path.display()
            ));
        }
        None => return ExitCode::SUCCESS,
    }

    match callbacks.manifest.finish() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => failure(format!(
            "failed to finish {}: {error}",
            invocation.manifest_path.display()
        )),
    }
}

fn prepare_driver_invocation() -> Result<DriverInvocation, String> {
    let arguments = utf8_compiler_arguments()?;
    let rustc = arguments
        .first()
        .ok_or_else(|| "optic rustc driver must receive rustc as its first argument, got none".to_owned())?;
    validate_compiler_command(rustc)?;

    let identity = expected_compiler()?;
    let manifest_path = required_path_environment(MANIFEST_PATH_ENV)?;

    Ok(DriverInvocation {
        arguments,
        identity,
        manifest_path,
    })
}

fn utf8_compiler_arguments() -> Result<Vec<String>, String> {
    env::args_os()
        .skip(1)
        .map(|argument| {
            argument.into_string().map_err(|_| {
                "optic rustc driver requires UTF-8 compiler arguments, got a non-UTF-8 argument"
                    .to_owned()
            })
        })
        .collect()
}

fn expected_compiler() -> Result<CompilerIdentity, String> {
    Ok(CompilerIdentity {
        rustc: required_environment(RUSTC_PATH_ENV)?,
        release: required_environment(RUSTC_RELEASE_ENV)?,
        commit_hash: required_environment(RUSTC_COMMIT_ENV)?,
        host: required_environment(RUSTC_HOST_ENV)?,
        sysroot: required_environment(RUSTC_SYSROOT_ENV)?,
    })
}

fn required_environment(name: &str) -> Result<String, String> {
    env::var(name).map_err(|error| format!("failed to read {name}, got {error}"))
}

fn validate_compiler_command(rustc: &str) -> Result<(), String> {
    let expected_command = required_path_environment(RUSTC_COMMAND_ENV)?;
    let expected_rustc = required_path_environment(RUSTC_PATH_ENV)?;
    let actual = resolve_compiler_command(rustc)?;
    let expected_command = canonicalize_compiler_path(&expected_command, "prepared rustc command")?;
    let expected_rustc = canonicalize_compiler_path(&expected_rustc, "prepared sysroot rustc")?;

    if actual != expected_command && actual != expected_rustc {
        return Err(format!(
            "selected rustc command must match the prepared compiler at {} or {}, got {}",
            expected_command.display(),
            expected_rustc.display(),
            actual.display(),
        ));
    }

    // A retained wrapper can change rustup's selection without changing the proxy path.
    // Probe in this inner environment before replacing that compiler with the prepared driver.
    let verbose = query_compiler(rustc, &["-vV"])?;
    let commit = verbose
        .lines()
        .find_map(|line| line.strip_prefix("commit-hash: "))
        .ok_or_else(|| "selected rustc -vV must report commit-hash, got no value".to_owned())?;
    let expected_commit = required_environment(RUSTC_COMMIT_ENV)?;
    if commit != expected_commit {
        return Err(format!(
            "selected rustc commit must match the prepared compiler {expected_commit}, got {commit}"
        ));
    }

    let sysroot = query_compiler(rustc, &["--print", "sysroot"])?;
    let sysroot = canonicalize_compiler_path(Path::new(sysroot.trim()), "selected rustc sysroot")?;
    let expected_sysroot = required_path_environment(RUSTC_SYSROOT_ENV)?;
    let expected_sysroot = canonicalize_compiler_path(&expected_sysroot, "prepared rustc sysroot")?;
    if sysroot != expected_sysroot {
        return Err(format!(
            "selected rustc sysroot must match the prepared compiler {}, got {}",
            expected_sysroot.display(),
            sysroot.display(),
        ));
    }

    Ok(())
}

fn resolve_compiler_command(rustc: &str) -> Result<PathBuf, String> {
    let path = Path::new(rustc);
    if path.components().count() > 1 || path.is_absolute() {
        return canonicalize_compiler_path(path, "selected rustc command");
    }

    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(path);
        #[cfg(windows)]
        let candidate = if candidate.extension().is_none() {
            candidate.with_extension("exe")
        } else {
            candidate
        };
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }

        return canonicalize_compiler_path(&candidate, "selected rustc command");
    }

    Err(format!(
        "selected rustc command must resolve to an executable through PATH, got {rustc}"
    ))
}

fn query_compiler(rustc: &str, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(rustc)
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to query selected rustc {rustc}, got {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "selected rustc {rustc} query must succeed, got {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|error| format!("selected rustc query must return UTF-8, got {error}"))
}

fn required_path_environment(name: &str) -> Result<PathBuf, String> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} must be set, got no value"))
}

fn canonicalize_compiler_path(path: &Path, field: &str) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {field} {}: {error}", path.display()))
}

fn run_wrapper() -> ExitCode {
    let mut arguments = env::args_os().skip(1).collect::<Vec<_>>();

    // Cargo puts the workspace wrapper before rustc. Its recorded presence identifies rustc without
    // inspecting path names.
    let compiler_index = usize::from(env::var_os(WORKSPACE_WRAPPER_ENV).is_some());
    if arguments.len() <= compiler_index {
        return failure(format!(
            "optic rustc wrapper must receive a compiler path at argument {compiler_index}, got {} arguments",
            arguments.len()
        ));
    }

    let Some(selected_target_marker) = env::var_os(SELECTED_TARGET_MARKER_ENV) else {
        return failure(format!("{SELECTED_TARGET_MARKER_ENV} is not set"));
    };
    let selected_target_marker = match selected_target_marker.into_string() {
        Ok(marker) => marker,
        Err(marker) => {
            return failure(format!(
                "{SELECTED_TARGET_MARKER_ENV} must be valid UTF-8, got {marker:?}"
            ));
        }
    };
    let Some(manifest_path) = env::var_os(MANIFEST_PATH_ENV).map(PathBuf::from) else {
        return failure(format!("{MANIFEST_PATH_ENV} is not set"));
    };
    let Some(collection_directory) = manifest_path.parent() else {
        return failure(format!(
            "{MANIFEST_PATH_ENV} must have a parent directory, got {}",
            manifest_path.display()
        ));
    };

    // Cargo can append target-specific rustc arguments after the arguments supplied after `--`.
    // Find the private marker by value instead of assuming that it remains the final argument.
    // Keep a response-file command compact because expanding it can exceed the same
    // operating-system limit that caused Cargo to create the file.
    let selected_target = match remove_selected_target_marker(
        &mut arguments,
        &selected_target_marker,
        collection_directory,
    ) {
        Ok(selected_target) => selected_target,
        Err(error) => return failure(error),
    };

    if selected_target {
        // Replace rustc only for the selected target. Existing wrappers keep their Cargo-defined
        // positions.
        let current_executable = match env::current_exe() {
            Ok(path) => path.into_os_string(),
            Err(error) => {
                return failure(format!(
                    "failed to find the optic rustc driver path: {error}"
                ));
            }
        };
        arguments.insert(compiler_index, current_executable);
    }

    if let Some(wrapper) = env::var_os(ORIGINAL_WRAPPER_ENV) {
        arguments.insert(0, wrapper);
    }
    let program = arguments.remove(0);
    let mut command = Command::new(program);
    command.args(arguments);
    if selected_target {
        command.env(DRIVER_INNER_ENV, "1");
    }

    execute(command)
}

#[cfg(unix)]
fn execute(mut command: Command) -> ExitCode {
    use std::os::unix::process::CommandExt;

    let error = command.exec();

    failure(format!("failed to start compiler wrapper, got {error}"))
}

#[cfg(not(unix))]
fn execute(mut command: Command) -> ExitCode {
    match command.status() {
        Ok(status) => {
            let Some(code) = status.code() else {
                return ExitCode::FAILURE;
            };
            let Ok(code) = u8::try_from(code) else {
                return ExitCode::FAILURE;
            };

            ExitCode::from(code)
        }
        Err(error) => failure(format!("failed to start compiler wrapper, got {error}")),
    }
}

fn failure(message: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", message.as_ref());

    ExitCode::FAILURE
}

/// Writes a bounded manifest that becomes visible only after successful collection.
struct ManifestWriter {
    /// The completed manifest path published by [`Self::finish`].
    path: PathBuf,

    /// The incomplete path that remains invisible to the parent process.
    temporary_path: PathBuf,

    /// The buffered protocol output.
    file: BufWriter<File>,

    /// The aggregate byte count enforced before each write.
    bytes_written: u64,

    /// The aggregate placement count enforced before each record.
    placement_count: usize,
}

impl ManifestWriter {
    fn create(path: &Path, compiler: &CompilerIdentity) -> io::Result<Self> {
        let temporary_path = path.with_extension("tmp");
        let file = BufWriter::new(File::create(&temporary_path)?);
        let mut writer = Self {
            path: path.to_owned(),
            temporary_path,
            file,
            bytes_written: 0,
            placement_count: 0,
        };
        writer.write_bytes(MANIFEST_MAGIC)?;
        writer.write_u32(PROTOCOL_VERSION)?;
        writer.write_string(&compiler.rustc)?;
        writer.write_string(&compiler.release)?;
        writer.write_string(&compiler.commit_hash)?;
        writer.write_string(&compiler.host)?;
        writer.write_string(&compiler.sysroot)?;

        Ok(writer)
    }

    fn write_placement(
        &mut self,
        instance: &ConcreteInstance,
        placement: &Placement,
    ) -> io::Result<()> {
        self.placement_count = self.placement_count.saturating_add(1);
        if self.placement_count > MAX_PLACEMENTS {
            return Err(invalid_data(format!(
                "placement count must not exceed {MAX_PLACEMENTS}, got {}",
                self.placement_count
            )));
        }

        self.write_u32(PLACEMENT_RECORD)?;
        self.write_string(&instance.definition_crate)?;
        self.write_string(&instance.definition_path)?;
        self.write_string(&instance.display_name)?;
        self.write_string(&instance.raw_symbol)?;
        self.write_string(&placement.codegen_unit)?;
        self.write_string(&placement.linkage)?;
        self.write_string(&placement.visibility)?;
        self.write_u32(u32::from(placement.local_copy))?;
        self.write_u64(u64::try_from(placement.size_estimate).map_err(|_| {
            invalid_data(format!(
                "placement size estimate must fit in u64, got {}",
                placement.size_estimate
            ))
        })?)
    }

    fn finish(mut self) -> io::Result<()> {
        self.write_u32(END_RECORD)?;
        self.file.flush()?;

        // Windows requires the file handle to close before the atomic rename.
        drop(self.file);
        fs::rename(self.temporary_path, self.path)
    }

    fn write_string(&mut self, value: &str) -> io::Result<()> {
        if value.len() > MAX_STRING_BYTES {
            return Err(invalid_data(format!(
                "string length must not exceed {MAX_STRING_BYTES}, got {}",
                value.len()
            )));
        }

        let length = u32::try_from(value.len()).map_err(|_| {
            invalid_data(format!(
                "string length must fit in u32, got {}",
                value.len()
            ))
        })?;
        self.write_u32(length)?;
        self.write_bytes(value.as_bytes())
    }

    fn write_u32(&mut self, value: u32) -> io::Result<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_u64(&mut self, value: u64) -> io::Result<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        let byte_count = u64::try_from(bytes.len()).map_err(|_| {
            invalid_data(format!("byte count must fit in u64, got {}", bytes.len()))
        })?;
        let next_length = self
            .bytes_written
            .checked_add(byte_count)
            .ok_or_else(|| {
                invalid_data(format!(
                    "manifest byte length must fit in u64, got overflow from {} + {byte_count}",
                    self.bytes_written
                ))
            })?;
        if next_length > MAX_MANIFEST_BYTES {
            return Err(invalid_data(format!(
                "manifest length must not exceed {MAX_MANIFEST_BYTES}, got {next_length}"
            )));
        }

        self.file.write_all(bytes)?;
        self.bytes_written = next_length;

        Ok(())
    }
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
