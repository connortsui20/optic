//! Selects an instance by its immutable position in one capture.
//!
//! References retain meaning only within that exact capture and format. Search ordering never
//! changes the ordinal. Resolving a reference must check the ordinal against its manifest.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

use crate::CaptureId;
use crate::Error;
use crate::error::InvalidFieldSnafu;

/// A full capture ID and a zero-based instance ordinal.
///
/// The canonical text is `<full-reverse-hex-capture-id>:<decimal-ordinal>`. Serde uses this same
/// string representation. Parsing rejects signs, whitespace, and leading ordinal zeros except `0`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InstanceRef {
    capture_id: CaptureId,
    ordinal: u64,
}

impl InstanceRef {
    /// Selects an ordinal without performing storage lookup.
    pub fn new(capture_id: CaptureId, ordinal: u64) -> Self {
        Self {
            capture_id,
            ordinal,
        }
    }

    /// Returns the capture that owns the referenced manifest.
    pub fn capture_id(&self) -> &CaptureId {
        &self.capture_id
    }

    /// Returns the immutable position before sorting or limiting search results.
    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }
}

impl fmt::Display for InstanceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.capture_id, self.ordinal)
    }
}

impl FromStr for InstanceRef {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let invalid = || {
            InvalidFieldSnafu {
                field: "instance reference",
                actual: value,
            }
            .build()
        };
        let (capture, ordinal) = value.split_once(':').ok_or_else(invalid)?;
        if ordinal.is_empty()
            || !ordinal.bytes().all(|byte| byte.is_ascii_digit())
            || (ordinal.len() > 1 && ordinal.starts_with('0'))
        {
            return Err(invalid());
        }

        Ok(Self::new(
            capture.parse()?,
            ordinal.parse().map_err(|_| invalid())?,
        ))
    }
}

impl Serialize for InstanceRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for InstanceRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
