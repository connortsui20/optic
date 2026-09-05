//! Runs the compiler wrapper that collects concrete instances for one selected Cargo target.
//!
//! Cargo invokes this executable once for each rustc process in the build. Ordinary invocations
//! enter [`wrapper::run`] and are replaced by the original rustc process without inspection. The
//! selected probe writes a stale receipt and exits before rustc analysis. A selected collection
//! contains a private marker argument. The wrapper removes that marker and starts this executable
//! again with [`protocol::DRIVER_INNER_ENV`] set. The second invocation
//! enters [`analysis::run`] and drives rustc. Its private manifest contains monomorphized functions,
//! normalized source relationships, effective codegen configuration, and expected optimized modules.
//!
//! This two-stage entry point keeps Cargo's wrapper calling convention out of the rustc callback.
//! The process boundary also lets ordinary compiler invocations use `exec`, so the wrapper does not
//! remain alive while rustc runs.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

mod analysis;
mod llvm;
mod manifest;
mod protocol;
mod source;
mod wrapper;

use std::env;
use std::process::ExitCode;

use protocol::DRIVER_INNER_ENV;

/// Selects the outer Cargo-wrapper entry point or the inner rustc-driver entry point.
fn main() -> ExitCode {
    if env::args_os().skip(1).eq([protocol::DRIVER_KEY_ARGUMENT]) {
        println!("{}", env!("OPTIC_DRIVER_KEY"));

        return ExitCode::SUCCESS;
    }

    if env::var_os(DRIVER_INNER_ENV).is_some() {
        analysis::run()
    } else {
        wrapper::run()
    }
}

/// Reports a driver-owned failure after rustc has no diagnostic to provide.
pub(crate) fn failure(message: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", message.as_ref());

    ExitCode::FAILURE
}
