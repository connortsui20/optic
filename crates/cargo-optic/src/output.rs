//! Adapts product records for human-readable command output.
//!
//! This separate view keeps display choices out of the durable format. Construction resolves
//! fallible timestamp formatting so [`CaptureOutput`] can implement [`std::fmt::Display`]. Instance
//! output exposes concrete instance evidence without adding presentation choices to the evidence
//! API.

use std::fmt;

use optic::CaptureId;
use optic::CaptureRecord;
use optic::InstanceRecord;
use snafu::ResultExt;
use snafu::Snafu;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A prevalidated human-readable view of one capture record.
pub(crate) struct CaptureOutput<'a> {
    /// The heading that distinguishes a new capture from a history entry.
    title: &'static str,
    /// The record whose stable fields this adapter renders.
    capture: &'a CaptureRecord,
    /// The preformatted completion time, stored to keep [`fmt::Display`] infallible apart from the
    /// formatter itself.
    completed: String,
}

impl<'a> CaptureOutput<'a> {
    pub(crate) fn new(title: &'static str, capture: &'a CaptureRecord) -> Result<Self, Error> {
        let nanoseconds = i128::from(capture.completed_at_unix_ms()) * 1_000_000;
        let completed = OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
            .context(TimestampSnafu)?
            .format(&Rfc3339)
            .context(TimestampFormatSnafu)?;

        Ok(Self {
            title,
            capture,
            completed,
        })
    }
}

impl fmt::Display for CaptureOutput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let build = self.capture.build();
        let target = build.target();

        writeln!(formatter, "{} {}", self.title, self.capture.id())?;
        writeln!(formatter, "  Completed  {}", self.completed)?;
        writeln!(
            formatter,
            "  Package    {} {}",
            build.package(),
            build.package_version(),
        )?;
        writeln!(
            formatter,
            "  Target     {} {}",
            target.kind(),
            target.name(),
        )?;
        writeln!(formatter, "  Profile    {}", build.profile())?;
        writeln!(formatter, "  Instances  {}", self.capture.instance_count())?;
        writeln!(formatter, "  Placements {}", self.capture.placement_count())
    }
}

/// A human-readable view of one concrete compiler instance.
pub(crate) struct InstanceOutput<'a> {
    /// The capture that contains the instance.
    capture_id: &'a CaptureId,
    /// The concrete compiler instance to render.
    instance: &'a InstanceRecord,
}

impl<'a> InstanceOutput<'a> {
    pub(crate) fn new(capture_id: &'a CaptureId, instance: &'a InstanceRecord) -> Self {
        Self {
            capture_id,
            instance,
        }
    }
}

impl fmt::Display for InstanceOutput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "Instance {}", self.instance.display_name())?;
        writeln!(formatter, "  Capture     {}", self.capture_id)?;
        writeln!(
            formatter,
            "  Definition  {}",
            self.instance.definition().definition_path()
        )?;
        writeln!(formatter, "  Symbol      {}", self.instance.raw_symbol())?;

        for placement in self.instance.placements() {
            writeln!(
                formatter,
                "  Placement   {}; linkage={}; visibility={}; local-copy={}; size={}",
                placement.codegen_unit(),
                placement.linkage(),
                placement.visibility(),
                placement.local_copy(),
                placement.size_estimate(),
            )?;
        }

        Ok(())
    }
}

/// Explains why a capture record could not become human-readable output.
#[derive(Debug, Snafu)]
pub(crate) enum Error {
    /// The recorded timestamp was outside the formatter's supported range.
    #[snafu(display("capture timestamp must be representable"))]
    Timestamp {
        /// The timestamp conversion error.
        source: time::error::ComponentRange,
    },
    /// The completion time could not use the required RFC 3339 representation.
    #[snafu(display("capture timestamp must use RFC 3339"))]
    TimestampFormat {
        /// The timestamp formatting error.
        source: time::error::Format,
    },
}
