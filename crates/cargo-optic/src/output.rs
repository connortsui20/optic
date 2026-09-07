//! Adapts product records for human-readable command output.
//!
//! This separate view keeps display choices out of the durable format. Construction resolves
//! fallible timestamp formatting so [`CaptureOutput`] can implement [`std::fmt::Display`]. Instance
//! output exposes concrete instance evidence without adding presentation choices to the evidence
//! API.

use std::fmt;

use optic::CaptureRecord;
use optic::FoundInstance;
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
    /// Formats the completion time before the capture is displayed.
    ///
    /// Returns an error if the recorded timestamp cannot use the required RFC 3339 representation.
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

        writeln!(formatter, "  Profile    {}", build.profile())
    }
}

/// A human-readable view of one concrete compiler instance.
pub(crate) struct InstanceOutput<'a> {
    /// The concrete compiler instance and its immutable reference.
    found: &'a FoundInstance,
}

impl<'a> InstanceOutput<'a> {
    /// Borrows one search result for command output.
    pub(crate) fn new(found: &'a FoundInstance) -> Self {
        Self { found }
    }
}

impl fmt::Display for InstanceOutput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let instance = self.found.record();

        writeln!(formatter, "Instance {}", instance.display_name())?;
        writeln!(formatter, "  Reference   {}", self.found.reference())?;
        writeln!(
            formatter,
            "  Capture     {}",
            self.found.reference().capture_id()
        )?;
        writeln!(
            formatter,
            "  Definition  {}",
            instance.definition().definition_path()
        )?;
        writeln!(formatter, "  Symbol      {}", instance.raw_symbol())?;

        for placement in instance.placements() {
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
