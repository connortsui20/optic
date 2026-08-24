//! Defines the complete durable entry published for one successful capture.
//!
//! [`CaptureRecord`] joins a stable [`CaptureId`] and completion time to an already validated
//! [`BuildRecord`] and [`ToolchainRecord`]. Construction always writes the current format version
//! and canonical capture ID. Deserialization rejects versions this crate does not understand and
//! normalizes the legacy capture ID spelling written by initial format-version-1 stores.

use serde::Deserialize;
use serde::Serialize;
use snafu::ensure;

use crate::BuildRecord;
use crate::CAPTURE_FORMAT_VERSION;
use crate::CaptureId;
use crate::Error;
use crate::ToolchainRecord;
use crate::UnsupportedFormatSnafu;

/// One immutable entry in completed capture history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedCaptureRecord")]
pub struct CaptureRecord {
    format_version: u32,
    id: CaptureId,
    completed_at_unix_ms: u64,
    build: BuildRecord,
    toolchain: ToolchainRecord,
}

impl CaptureRecord {
    /// Creates a capture using the format version understood by this release.
    #[must_use]
    pub fn new(
        id: CaptureId,
        completed_at_unix_ms: u64,
        build: BuildRecord,
        toolchain: ToolchainRecord,
    ) -> Self {
        Self {
            format_version: CAPTURE_FORMAT_VERSION,
            id,
            completed_at_unix_ms,
            build,
            toolchain,
        }
    }

    /// Returns the durable record format version.
    #[must_use]
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the stable capture identity.
    #[must_use]
    pub fn id(&self) -> &CaptureId {
        &self.id
    }

    /// Returns the completion time as Unix milliseconds.
    #[must_use]
    pub fn completed_at_unix_ms(&self) -> u64 {
        self.completed_at_unix_ms
    }

    /// Returns the Cargo invocation recorded for this capture.
    #[must_use]
    pub fn build(&self) -> &BuildRecord {
        &self.build
    }

    /// Returns the compiler identity recorded for this capture.
    #[must_use]
    pub fn toolchain(&self) -> &ToolchainRecord {
        &self.toolchain
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedCaptureRecord {
    format_version: u32,
    id: String,
    completed_at_unix_ms: u64,
    build: BuildRecord,
    toolchain: ToolchainRecord,
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
        let id = CaptureId::from_storage_str(&record.id)?;

        Ok(Self::new(
            id,
            record.completed_at_unix_ms,
            record.build,
            record.toolchain,
        ))
    }
}
