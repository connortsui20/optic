//! Discovers the active nightly compiler and its matching LLVM tools.
//!
//! The active toolchain is the one selected by the caller, such as through `cargo +nightly optic`.
//! Optic rejects stable compilers and system `llvm-dis` binaries because bitcode compatibility is
//! tied to the LLVM version embedded in rustc.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The exact active compiler and matching LLVM disassembler.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Toolchain {
    /// The rustc release string.
    pub release: String,

    /// The rustc commit hash.
    pub commit_hash: String,

    /// The compiler host triple.
    pub host: String,

    /// The embedded LLVM version.
    pub llvm_version: String,

    /// The active compiler sysroot.
    pub sysroot: PathBuf,

    /// The matching `llvm-dis` executable.
    pub llvm_dis: PathBuf,
}

/// Inspects the active rustc and validates the first-release toolchain contract.
///
/// # Errors
///
/// Returns an error if rustc fails, the active compiler is not nightly, or its matching
/// `llvm-dis` executable is absent.
pub fn inspect_toolchain() -> Result<Toolchain> {
    let verbose = run_rustc(["-vV"])?;
    let release = field(&verbose, "release")?.to_owned();

    if !release.contains("nightly") {
        return Err(Error::StableCompiler { release });
    }

    let commit_hash = field(&verbose, "commit-hash")?.to_owned();
    let host = field(&verbose, "host")?.to_owned();
    let llvm_version = field(&verbose, "LLVM version")?.to_owned();
    let sysroot = PathBuf::from(run_rustc(["--print", "sysroot"])?.trim());
    let executable = if cfg!(windows) {
        "llvm-dis.exe"
    } else {
        "llvm-dis"
    };
    let llvm_dis = sysroot
        .join("lib")
        .join("rustlib")
        .join(&host)
        .join("bin")
        .join(executable);

    if !llvm_dis.is_file() {
        return Err(Error::MissingLlvmDis { path: llvm_dis });
    }

    Ok(Toolchain {
        release,
        commit_hash,
        host,
        llvm_version,
        sysroot,
        llvm_dis,
    })
}

fn run_rustc<const N: usize>(arguments: [&str; N]) -> Result<String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let program = rustc.to_string_lossy().into_owned();
    let output = Command::new(rustc)
        .args(arguments)
        .output()
        .map_err(|source| Error::StartProcess {
            program: program.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program,
            status: output.status.to_string(),
            diagnostics: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn field<'a>(verbose: &'a str, name: &'static str) -> Result<&'a str> {
    verbose
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .ok_or(Error::MissingToolchainField { field: name })
}
