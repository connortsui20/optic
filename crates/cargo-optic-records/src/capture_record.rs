//! Defines the complete durable entry published for one successful capture.
//!
//! [`CaptureRecord`] joins a stable [`CaptureId`] and completion time to an already validated
//! [`BuildRecord`]. Construction always writes the current format version and canonical capture ID.
//! Deserialization rejects versions that this crate does not understand.

use serde::Deserialize;
use serde::Serialize;
use snafu::ensure;

use crate::BuildRecord;
use crate::CAPTURE_FORMAT_VERSION;
use crate::CaptureId;
use crate::Error;
use crate::error::UnsupportedFormatSnafu;

/// One immutable entry in completed capture history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedCaptureRecord")]
pub struct CaptureRecord {
    format_version: u32,
    id: CaptureId,
    completed_at_unix_ms: u64,
    build: BuildRecord,
}

impl CaptureRecord {
    /// Creates a capture using the format version understood by this release.
    pub fn new(id: CaptureId, completed_at_unix_ms: u64, build: BuildRecord) -> Self {
        Self {
            format_version: CAPTURE_FORMAT_VERSION,
            id,
            completed_at_unix_ms,
            build,
        }
    }

    /// Returns the durable record format version.
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the stable capture identity.
    pub fn id(&self) -> &CaptureId {
        &self.id
    }

    /// Returns the completion time as Unix milliseconds.
    pub fn completed_at_unix_ms(&self) -> u64 {
        self.completed_at_unix_ms
    }

    /// Returns the Cargo invocation recorded for this capture.
    pub fn build(&self) -> &BuildRecord {
        &self.build
    }
}

/// The serialized fields that must pass [`CaptureRecord`] validation during deserialization.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedCaptureRecord {
    format_version: u32,
    id: CaptureId,
    completed_at_unix_ms: u64,
    build: BuildRecord,
}

impl TryFrom<UncheckedCaptureRecord> for CaptureRecord {
    type Error = Error;

    fn try_from(record: UncheckedCaptureRecord) -> Result<Self, Self::Error> {
        ensure!(
            record.format_version == CAPTURE_FORMAT_VERSION,
            UnsupportedFormatSnafu {
                expected: CAPTURE_FORMAT_VERSION,
                actual: record.format_version,
            }
        );
        Ok(Self::new(
            record.id,
            record.completed_at_unix_ms,
            record.build,
        ))
    }
}
