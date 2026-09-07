//! Identifies the normalized request used to select a capture candidate.
//!
//! The compiler uses [`CaptureKey`] to locate completed captures before it asks Cargo to verify
//! freshness.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::Error;
use crate::error::InvalidFieldSnafu;

/// A SHA-256 digest encoded as exactly 64 lowercase hexadecimal characters.
///
/// The compiler hashes length-prefixed request inputs, including its request-policy revision.
/// Parsing checks only the digest text. A matching key selects a candidate for Cargo verification.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CaptureKey(String);

impl CaptureKey {
    /// Returns the canonical digest text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CaptureKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for CaptureKey {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return InvalidFieldSnafu {
                field: "capture key (64 lowercase hexadecimal characters)",
                actual: value,
            }
            .fail();
        }

        Ok(Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for CaptureKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
