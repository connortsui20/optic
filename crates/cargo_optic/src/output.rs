//! Renders application records for people using the command line.
//!
//! [`CaptureOutput`] is a display adapter rather than a canonical representation of
//! [`CaptureRecord`]. The durable JSON record and this terminal view serve different compatibility
//! contracts: storage must round-trip every field, while the terminal view can select and label the
//! fields useful during inspection.
//!
//! Timestamp conversion is fallible, so construction performs it before formatting begins. Once a
//! [`CaptureOutput`] exists, its [`std::fmt::Display`] implementation has only ordinary formatter
//! failures and can be passed directly to `print!` or `write!`.

use std::fmt;

use optic::CaptureRecord;
use snafu::ResultExt;
use snafu::Snafu;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub(crate) struct CaptureOutput<'a> {
    title: &'static str,
    capture: &'a CaptureRecord,
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
        let toolchain = self.capture.toolchain();

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
        writeln!(formatter, "  rustc      {}", toolchain.release())?;
        writeln!(formatter, "  Commit     {}", toolchain.commit_hash())?;
        writeln!(formatter, "  Host       {}", toolchain.host())?;
        writeln!(formatter, "  LLVM       {}", toolchain.llvm_version())
    }
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("capture timestamp must be representable"))]
    Timestamp { source: time::error::ComponentRange },

    #[snafu(display("capture timestamp must use RFC 3339"))]
    TimestampFormat { source: time::error::Format },
}
