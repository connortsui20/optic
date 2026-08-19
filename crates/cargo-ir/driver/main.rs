//! Collects compiler-owned function identities during one rustc invocation.
//!
//! This executable is both Cargo's outer rustc wrapper and the compiler driver for the selected
//! target. Other invocations continue through the original wrapper chain without compiler queries.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::Compiler;
use rustc_middle::mono::MonoItem;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::print::with_no_trimmed_paths;

const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_ID\0\0";
const PROTOCOL_VERSION: u32 = 1;
const MANIFEST_PATH_ENV: &str = "OPTIC_IDENTITY_MANIFEST";
const ORIGINAL_WRAPPER_ENV: &str = "OPTIC_ORIGINAL_RUSTC_WRAPPER";
const RUSTC_COMMIT_ENV: &str = "OPTIC_RUSTC_COMMIT";
const SELECTED_TEMPS_ENV: &str = "OPTIC_SELECTED_TEMPS_DIR";
const WORKSPACE_WRAPPER_ENV: &str = "OPTIC_HAS_WORKSPACE_WRAPPER";
const WRAPPER_ACTIVE_ENV: &str = "OPTIC_RUSTC_WRAPPER_ACTIVE";
const DRIVER_INNER_ENV: &str = "OPTIC_RUSTC_DRIVER_INNER";

#[derive(Default)]
struct IdentityCallbacks {
    identities: BTreeMap<String, CompilerIdentity>,
}

struct CompilerIdentity {
    definition: String,
    display_name: String,
    raw_symbol: String,
    codegen_units: Vec<String>,
}

impl Callbacks for IdentityCallbacks {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let partitions = tcx.collect_and_partition_mono_items(());

        for codegen_unit in partitions.codegen_units {
            for (mono_item, _) in codegen_unit.items_in_deterministic_order(tcx) {
                let MonoItem::Fn(instance) = mono_item else {
                    continue;
                };
                let raw_symbol = tcx.symbol_name(instance).name.to_owned();
                let identity = self
                    .identities
                    .entry(raw_symbol.clone())
                    .or_insert_with(|| CompilerIdentity {
                        definition: tcx.def_path_str(instance.def_id()),
                        display_name: with_no_trimmed_paths!(
                            tcx.def_path_str_with_args(instance.def_id(), instance.args)
                        ),
                        raw_symbol,
                        codegen_units: Vec::new(),
                    });
                identity.codegen_units.push(codegen_unit.name().to_string());
            }
        }

        Compilation::Continue
    }
}

fn main() -> ExitCode {
    if env::var_os(WRAPPER_ACTIVE_ENV).is_some() && env::var_os(DRIVER_INNER_ENV).is_none() {
        return run_wrapper();
    }

    if env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--optic-version")
    {
        println!("{PROTOCOL_VERSION}");

        return ExitCode::SUCCESS;
    }

    run_driver()
}

fn run_driver() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() {
        eprintln!("optic rustc driver requires the rustc path as its first argument");

        return ExitCode::FAILURE;
    }

    let mut callbacks = IdentityCallbacks::default();
    let exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&arguments, &mut callbacks);
    });
    if exit_code != ExitCode::SUCCESS {
        return exit_code;
    }

    let manifest_path = match env::var_os(MANIFEST_PATH_ENV) {
        Some(path) => PathBuf::from(path),
        None => {
            eprintln!("{MANIFEST_PATH_ENV} is not set");

            return ExitCode::FAILURE;
        }
    };
    let rustc_commit = match env::var(RUSTC_COMMIT_ENV) {
        Ok(commit) => commit,
        Err(error) => {
            eprintln!("failed to read {RUSTC_COMMIT_ENV}: {error}");

            return ExitCode::FAILURE;
        }
    };

    match write_manifest(&manifest_path, &rustc_commit, callbacks.identities.values()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("failed to write {}: {error}", manifest_path.display());

            ExitCode::FAILURE
        }
    }
}

fn run_wrapper() -> ExitCode {
    let mut arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let has_workspace_wrapper = env::var_os(WORKSPACE_WRAPPER_ENV).is_some();
    let compiler_index = if has_workspace_wrapper { 1 } else { 0 };
    if arguments.len() <= compiler_index {
        eprintln!("optic rustc wrapper did not receive a compiler path");

        return ExitCode::FAILURE;
    }

    let selected_temps = match env::var_os(SELECTED_TEMPS_ENV) {
        Some(path) => path,
        None => {
            eprintln!("{SELECTED_TEMPS_ENV} is not set");

            return ExitCode::FAILURE;
        }
    };
    let selected_argument = temps_argument(&selected_temps);
    let selected_target = arguments[compiler_index + 1..]
        .windows(2)
        .any(|pair| pair[0] == OsStr::new("-Z") && pair[1] == selected_argument);

    if selected_target {
        let rustc = arguments[compiler_index].clone();
        let current_executable = match env::current_exe() {
            Ok(path) => path.into_os_string(),
            Err(error) => {
                eprintln!("failed to find the optic rustc driver path: {error}");

                return ExitCode::FAILURE;
            }
        };
        arguments[compiler_index] = current_executable;
        arguments.insert(compiler_index + 1, rustc);
    }

    let mut command = if let Some(wrapper) = env::var_os(ORIGINAL_WRAPPER_ENV) {
        let mut command = Command::new(wrapper);
        command.args(arguments);
        command
    } else {
        let program = arguments.remove(0);
        let mut command = Command::new(program);
        command.args(arguments);
        command
    };
    if selected_target {
        command.env(DRIVER_INNER_ENV, "1");
    }

    execute(command)
}

fn temps_argument(path: &OsStr) -> OsString {
    let mut argument = OsString::from("temps-dir=");
    argument.push(path);

    argument
}

#[cfg(unix)]
fn execute(mut command: Command) -> ExitCode {
    use std::os::unix::process::CommandExt;

    let error = command.exec();
    eprintln!("failed to start compiler wrapper: {error}");

    ExitCode::FAILURE
}

#[cfg(not(unix))]
fn execute(mut command: Command) -> ExitCode {
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("failed to start compiler wrapper: {error}");

            ExitCode::FAILURE
        }
    }
}

fn write_manifest<'a>(
    path: &Path,
    rustc_commit: &str,
    identities: impl ExactSizeIterator<Item = &'a CompilerIdentity>,
) -> io::Result<()> {
    let temporary_path = path.with_extension("tmp");
    let mut file = File::create(&temporary_path)?;
    file.write_all(MANIFEST_MAGIC)?;
    file.write_all(&PROTOCOL_VERSION.to_le_bytes())?;
    write_string(&mut file, rustc_commit)?;
    write_u64(&mut file, identities.len())?;

    for identity in identities {
        write_string(&mut file, &identity.definition)?;
        write_string(&mut file, &identity.display_name)?;
        write_string(&mut file, &identity.raw_symbol)?;
        write_u32(&mut file, identity.codegen_units.len())?;

        for codegen_unit in &identity.codegen_units {
            write_string(&mut file, codegen_unit)?;
        }
    }

    file.sync_all()?;
    // Windows requires the file handle to close before an atomic rename.
    drop(file);
    fs::rename(temporary_path, path)
}

fn write_string(file: &mut File, value: &str) -> io::Result<()> {
    write_u32(file, value.len())?;
    file.write_all(value.as_bytes())
}

fn write_u32(file: &mut File, value: usize) -> io::Result<()> {
    let value = u32::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("value must fit in u32, got {value}"),
        )
    })?;
    file.write_all(&value.to_le_bytes())
}

fn write_u64(file: &mut File, value: usize) -> io::Result<()> {
    let value = u64::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("value must fit in u64, got {value}"),
        )
    })?;
    file.write_all(&value.to_le_bytes())
}
