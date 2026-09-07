//! Reuses validated Cargo evidence or atomically publishes one new capture.
//!
//! [`capture`] coordinates Cargo freshness and durable publication. A fresh request returns its
//! existing immutable record. Each actual compilation receives a new analysis token, so failed
//! publication cannot associate old evidence with a different build. Compiler execution, record
//! validation, and physical storage remain owned by their respective subsystems.

use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use optic_compiler::BuildRequest;
use optic_compiler::Freshness;
use optic_compiler::Workspace;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::InstanceManifest;
use optic_records::LlvmCollection;
use optic_store::Store;

mod error;
pub use error::Error;

/// Selects whether capture can reuse evidence that Cargo confirms as fresh.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CapturePolicy {
    /// Reuses a complete matching capture only after Cargo verifies freshness.
    #[default]
    Reuse,
    /// Forces new selected-target analysis while retaining compatible driver and dependency caches.
    Fresh,
}

/// Distinguishes newly published evidence from an unchanged completed capture.
#[derive(Debug)]
pub enum CaptureOutcome {
    /// Contains a new capture produced by successful analysis and atomic publication.
    Captured(CaptureRecord),
    /// Contains the original capture, including its unchanged identity and completion time.
    Reused(CaptureRecord),
}

impl CaptureOutcome {
    /// Returns the immutable record regardless of whether capture reused it.
    pub fn record(&self) -> &CaptureRecord {
        match self {
            Self::Captured(record) | Self::Reused(record) => record,
        }
    }

    /// Returns ownership of the completed capture record.
    pub fn into_record(self) -> CaptureRecord {
        match self {
            Self::Captured(record) | Self::Reused(record) => record,
        }
    }
}

/// Reuses a Cargo-fresh capture or collects and publishes new evidence.
///
/// The caller **must** serialize captures within the workspace and driver-cache provisioning within
/// its Cargo home. A reuse performs no durable writes. A failed new capture leaves previous
/// completed captures readable, but never falls back to their evidence as the operation's result.
///
/// # Errors
///
/// Returns an error if request preparation, candidate validation, freshness verification,
/// compiler collection, or publication fails.
pub fn capture(
    workspace: &Workspace,
    store: &Store,
    request: &BuildRequest,
    policy: CapturePolicy,
) -> Result<CaptureOutcome, Error> {
    let mut prepared = optic_compiler::prepare_build(workspace, request)?;
    store.initialize()?;

    if policy == CapturePolicy::Reuse
        && let Some((candidate, manifest)) = store.read_candidate(prepared.request_key())?
        && prepared.probe(candidate.analysis())? == Freshness::Fresh
    {
        warn_unavailable_llvm(&manifest);

        return Ok(CaptureOutcome::Reused(candidate));
    }

    let collected = prepared.collect()?;
    let capture_id = CaptureId::generate();
    let (build, compiler, analysis, manifest, artifacts) =
        collected.into_parts(capture_id.clone())?;

    let completed_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock must be after the Unix epoch")
        .as_millis()
        .try_into()
        .expect("the current Unix timestamp must fit in u64 milliseconds");
    let capture = CaptureRecord::new(capture_id, completed_at_unix_ms, build, compiler, analysis);

    store.publish(&capture, &manifest, artifacts.path())?;
    warn_unavailable_llvm(&manifest);

    Ok(CaptureOutcome::Captured(capture))
}

/// Reports the stored support boundary for both newly published and reused evidence.
fn warn_unavailable_llvm(manifest: &InstanceManifest) {
    if let LlvmCollection::NotCaptured(reason) = manifest.llvm() {
        eprintln!("warning: optimized LLVM was not captured: {reason}");
    }
}
