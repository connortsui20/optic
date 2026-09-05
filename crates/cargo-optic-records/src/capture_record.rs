//! Defines the complete durable entry published for one successful capture.
//!
//! [`CaptureRecord`] joins a stable [`CaptureId`], completion time, compiler identity, and a
//! validated [`BuildRecord`]. Construction writes the current format version and canonical capture
//! ID. Deserialization rejects versions that this crate does not understand.

use serde::Deserialize;
use serde::Serialize;
use snafu::ensure;

use crate::BuildRecord;
use crate::CAPTURE_FORMAT_VERSION;
use crate::CaptureId;
use crate::CompilerIdentity;
use crate::Error;
use crate::error::InvalidFieldSnafu;
use crate::error::UnsupportedFormatSnafu;

/// One immutable entry in completed capture history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawCaptureRecord")]
pub struct CaptureRecord {
    format_version: u32,
    id: CaptureId,
    completed_at_unix_ms: u64,
    build: BuildRecord,
    compiler: CompilerIdentity,
}

impl CaptureRecord {
    /// Creates a capture using the format version understood by this release.
    pub fn new(
        id: CaptureId,
        completed_at_unix_ms: u64,
        build: BuildRecord,
        compiler: CompilerIdentity,
    ) -> Self {
        Self {
            format_version: CAPTURE_FORMAT_VERSION,
            id,
            completed_at_unix_ms,
            build,
            compiler,
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

    /// Returns the compiler that ran the selected target invocation.
    pub fn compiler(&self) -> &CompilerIdentity {
        &self.compiler
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCaptureRecord {
    format_version: u32,
    id: CaptureId,
    completed_at_unix_ms: u64,
    build: BuildRecord,
    #[serde(default)]
    compiler: Option<CompilerIdentity>,
}

impl TryFrom<RawCaptureRecord> for CaptureRecord {
    type Error = Error;

    fn try_from(record: RawCaptureRecord) -> Result<Self, Self::Error> {
        ensure!(
            record.format_version == CAPTURE_FORMAT_VERSION,
            UnsupportedFormatSnafu {
                expected: CAPTURE_FORMAT_VERSION,
                actual: record.format_version,
            }
        );

        let compiler = record.compiler.ok_or_else(|| {
            InvalidFieldSnafu {
                field: "compiler",
                actual: "no value",
            }
            .build()
        })?;
        Ok(Self::new(
            record.id,
            record.completed_at_unix_ms,
            record.build,
            compiler,
        ))
    }
}
