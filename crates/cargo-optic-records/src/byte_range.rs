//! Represents finite byte ranges without narrowing large artifact offsets.
//!
//! Construction checks arithmetic. The manifest and store check the range against declared and
//! actual file lengths respectively.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::error::InvalidFieldSnafu;

/// A half-open byte range whose exclusive end fits in `u64`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawByteRange")]
pub struct ByteRange {
    start: u64,
    length: u64,
}

impl ByteRange {
    /// Creates a range, including empty ranges.
    ///
    /// # Errors
    ///
    /// Returns an error if `start + length` exceeds `u64::MAX`.
    pub fn new(start: u64, length: u64) -> Result<Self, Error> {
        if start.checked_add(length).is_none() {
            return InvalidFieldSnafu {
                field: "byte range",
                actual: format!("overflow at {start} + {length}"),
            }
            .fail();
        }

        Ok(Self { start, length })
    }

    /// Returns the inclusive byte offset.
    pub fn start(self) -> u64 {
        self.start
    }

    /// Returns the number of bytes.
    pub fn length(self) -> u64 {
        self.length
    }

    /// Returns the exclusive byte offset, checked by construction.
    pub fn end(self) -> u64 {
        self.start + self.length
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawByteRange {
    start: u64,
    length: u64,
}

impl TryFrom<RawByteRange> for ByteRange {
    type Error = Error;

    fn try_from(raw: RawByteRange) -> Result<Self, Error> {
        Self::new(raw.start, raw.length)
    }
}
