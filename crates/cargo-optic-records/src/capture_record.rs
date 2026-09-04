//! Defines the complete durable entry published for one successful capture.
//!
//! [`CaptureRecord`] joins a stable [`CaptureId`], completion time, compiler identity, a validated
//! [`BuildRecord`], and the evidence counts copied from its instance manifest. The counts let
//! history readers summarize a capture without parsing that potentially large manifest.
//! Construction writes the current format version and canonical capture ID. Deserialization
//! rejects versions that this crate does not understand.

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
    instance_count: u64,
    placement_count: u64,
}

impl CaptureRecord {
    /// Creates a capture using the format version understood by this release.
    ///
    /// # Errors
    ///
    /// Returns an error unless zero instances have zero placements and each nonempty set has at
    /// least one placement per instance.
    pub fn new(
        id: CaptureId,
        completed_at_unix_ms: u64,
        build: BuildRecord,
        compiler: CompilerIdentity,
        instance_count: u64,
        placement_count: u64,
    ) -> Result<Self, Error> {
        let counts_are_valid = match instance_count {
            0 => placement_count == 0,
            _ => placement_count >= instance_count,
        };
        if !counts_are_valid {
            return crate::error::InvalidFieldSnafu {
                field: "capture counts",
                actual: format!("{instance_count} instances and {placement_count} placements"),
            }
            .fail();
        }

        Ok(Self {
            format_version: CAPTURE_FORMAT_VERSION,
            id,
            completed_at_unix_ms,
            build,
            compiler,
            instance_count,
            placement_count,
        })
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

    /// Returns the number of concrete instances in this capture's manifest.
    pub fn instance_count(&self) -> u64 {
        self.instance_count
    }

    /// Returns the number of codegen-unit placements in this capture's manifest.
    pub fn placement_count(&self) -> u64 {
        self.placement_count
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
    #[serde(default)]
    instance_count: Option<u64>,
    #[serde(default)]
    placement_count: Option<u64>,
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
        let instance_count = record.instance_count.ok_or_else(|| {
            InvalidFieldSnafu {
                field: "instance count",
                actual: "no value",
            }
            .build()
        })?;
        let placement_count = record.placement_count.ok_or_else(|| {
            InvalidFieldSnafu {
                field: "placement count",
                actual: "no value",
            }
            .build()
        })?;

        Self::new(
            record.id,
            record.completed_at_unix_ms,
            record.build,
            compiler,
            instance_count,
            placement_count,
        )
    }
}
