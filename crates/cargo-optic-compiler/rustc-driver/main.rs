//! Runs the compiler wrapper that collects concrete instances for one selected Cargo target.
//!
//! Cargo invokes this executable once for each rustc process in the build. Ordinary invocations
//! enter [`wrapper::run`] and are replaced by the original rustc process without inspection. The
//! selected target contains a private marker argument, so the wrapper removes that marker and
//! starts this executable again with [`protocol::DRIVER_INNER_ENV`] set. The second invocation
//! enters [`analysis::run`], drives rustc, and writes the selected target's monomorphized functions
//! to the private manifest.
//!
//! This two-stage entry point keeps Cargo's wrapper calling convention out of the rustc callback.
//! The process boundary also lets ordinary compiler invocations use `exec`, so the wrapper does not
//! remain alive while rustc runs.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

mod analysis;
mod manifest;
mod protocol;
mod wrapper;

use std::env;
use std::process::ExitCode;

use protocol::DRIVER_INNER_ENV;

/// Selects the outer Cargo-wrapper entry point or the inner rustc-driver entry point.
fn main() -> ExitCode {
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
