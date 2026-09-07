//! Associates each instance with exact compiler-loaded source or a recorded absence.
//!
//! Several instances can share one artifact and range. Display paths provide context only and never
//! authorize filesystem reads from the current checkout.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::ArtifactId;
use crate::ByteRange;
use crate::Error;
use crate::error::InvalidFieldSnafu;
use crate::validation::require_path;

/// The source evidence recorded for one concrete instance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAvailability {
    /// A whole supported definition in compiler-loaded source.
    Available(SourceRecord),
    /// The compiler could not establish a supported exact source span.
    Unavailable(SourceUnavailable),
}

/// Why exact supported source is absent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceUnavailable {
    /// The definition belongs to another crate, including external path dependencies.
    Nonlocal,

    /// The compiler did not retain source text for the definition.
    Unloaded,

    /// The definition comes from an unsupported expansion or synthetic instance.
    Generated,

    /// The compiler could not establish one complete, valid source-file span.
    UnsupportedSpan,

    /// The source lies outside the canonical selected package root.
    OutsidePackage,
}

impl fmt::Display for SourceUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Nonlocal => "source is unavailable for a nonlocal definition",
            Self::Unloaded => "source text was not loaded by the compiler",
            Self::Generated => {
                "source is unavailable for an unsupported expansion or synthetic instance"
            }
            Self::UnsupportedSpan => "the compiler did not establish a supported exact source span",
            Self::OutsidePackage => "source lies outside the selected package root",
        })
    }
}

/// A source range and its original display location.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawSourceRecord")]
pub struct SourceRecord {
    artifact: ArtifactId,
    range: ByteRange,
    display_path: PathBuf,
    starting_line: u64,
}

impl SourceRecord {
    /// Associates a checked range with a source artifact.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty display path or a zero starting line.
    pub fn new(
        artifact: ArtifactId,
        range: ByteRange,
        display_path: PathBuf,
        starting_line: u64,
    ) -> Result<Self, Error> {
        require_path("source display path", &display_path)?;

        if starting_line == 0 {
            return InvalidFieldSnafu {
                field: "source starting line",
                actual: "zero",
            }
            .fail();
        }

        Ok(Self {
            artifact,
            range,
            display_path,
            starting_line,
        })
    }

    /// Returns the capture-local source file identity.
    pub fn artifact(&self) -> ArtifactId {
        self.artifact
    }

    /// Returns the range in normalized compiler-loaded UTF-8 bytes.
    pub fn range(&self) -> ByteRange {
        self.range
    }

    /// Returns the display path without authorizing a current-source read.
    pub fn display_path(&self) -> &Path {
        &self.display_path
    }

    /// Returns the one-based line containing the first byte of the range.
    pub fn starting_line(&self) -> u64 {
        self.starting_line
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSourceRecord {
    artifact: ArtifactId,
    range: ByteRange,
    display_path: PathBuf,
    starting_line: u64,
}

impl TryFrom<RawSourceRecord> for SourceRecord {
    type Error = Error;

    fn try_from(raw: RawSourceRecord) -> Result<Self, Error> {
        Self::new(raw.artifact, raw.range, raw.display_path, raw.starting_line)
    }
}
