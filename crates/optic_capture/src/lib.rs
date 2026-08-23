//! Coordinates the transition from a requested build to completed capture history.
//!
//! [`capture`] is the write-side workflow between the compiler and store crates. It first asks
//! [`optic_compiler`] to resolve and execute the request. Only after Cargo succeeds does it assign
//! a [`CaptureId`], read the completion time, and construct a [`CaptureRecord`]. The store then
//! atomically publishes that complete record.
//!
//! This ordering is the important boundary: failed builds have neither an ID nor a completed store
//! entry. Storage paths, command-line presentation, and workspace discovery remain outside this
//! crate.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use optic_compiler::BuildRequest;
use optic_compiler::Workspace;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_store::Store;
use snafu::ResultExt;
use snafu::Snafu;

/// Builds one target and publishes its completed capture metadata.
///
/// # Errors
///
/// Returns an error if compiler execution, clock conversion, or publication fails. No failed
/// operation becomes visible as a completed capture.
pub fn capture(
    workspace: &Workspace,
    store: &Store,
    request: &BuildRequest,
) -> Result<CaptureRecord, Error> {
    let completed = optic_compiler::run_build(workspace, request)?;
    let completed_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context(ClockSnafu)?
        .as_millis()
        .try_into()
        .context(TimestampOverflowSnafu)?;

    let record = CaptureRecord::new(
        CaptureId::new(),
        completed_at_unix_ms,
        completed.build().clone(),
        completed.toolchain().clone(),
    );

    store.publish(&record)?;

    Ok(record)
}

/// Explains why a requested build did not become a completed capture.
#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(transparent)]
    Compiler { source: optic_compiler::Error },

    #[snafu(display("system clock must be at or after the Unix epoch"))]
    Clock { source: std::time::SystemTimeError },

    #[snafu(display("capture timestamp must fit in u64 milliseconds, got an overflow"))]
    TimestampOverflow { source: std::num::TryFromIntError },

    #[snafu(transparent)]
    Store { source: optic_store::Error },
}
