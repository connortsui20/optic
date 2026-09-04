//! Collects and atomically publishes one complete capture.
//!
//! [`capture`] is the planning boundary between compiler collection and durable publication. It
//! assigns the capture identity and completion time only after the selected-target compiler
//! invocation returns validated evidence, then publishes the record and instance manifest through
//! one store operation. Compiler execution, record definitions, and physical storage remain owned
//! by their respective subsystems.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use optic_compiler::BuildRequest;
use optic_compiler::Workspace;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::InstanceManifest;
use optic_store::Store;
use snafu::ResultExt;

mod error;
pub use error::Error;

use error::ClockSnafu;
use error::TimestampOverflowSnafu;

/// Collects compiler evidence and publishes one complete capture.
///
/// # Errors
///
/// Returns an error if compiler collection fails, the completion timestamp cannot be represented,
/// the instance manifest is invalid, or the complete capture cannot be published.
pub fn capture(
    workspace: &Workspace,
    store: &Store,
    request: &BuildRequest,
) -> Result<CaptureRecord, Error> {
    let collected = optic_compiler::collect_build(workspace, request)?;
    let (build, compiler, instances) = collected.into_parts();

    let capture_id = CaptureId::generate();
    let instances = InstanceManifest::new(capture_id.clone(), instances)?;

    let completed_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context(ClockSnafu)?
        .as_millis();
    let completed_at_unix_ms = u64::try_from(completed_at_unix_ms).map_err(|_| {
        TimestampOverflowSnafu {
            actual: completed_at_unix_ms,
        }
        .build()
    })?;
    let capture = CaptureRecord::new(
        capture_id,
        completed_at_unix_ms,
        build,
        compiler,
        instances.instance_count(),
        instances.placement_count(),
    )?;

    store.publish(&capture, &instances)?;

    Ok(capture)
}
