//! Identifies one selected-target compilation independently of its request.
//!
//! Published tokens are only eligible for freshness probes. Each new compilation needs a newly
//! generated token, including attempts after failed publication.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::Error;
use crate::error::InvalidFieldSnafu;

/// A random UUID-v4 token encoded as 32 lowercase hexadecimal characters without separators.
///
/// Parsing validates both the canonical representation and the UUID version and variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AnalysisToken(String);

impl AnalysisToken {
    /// Generates the identity for a new compilation attempt.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }

    /// Returns the canonical token text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AnalysisToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for AnalysisToken {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let valid = value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.as_bytes()[12] == b'4'
            && matches!(value.as_bytes()[16], b'8' | b'9' | b'a' | b'b');

        if !valid {
            return InvalidFieldSnafu {
                field: "analysis token (32 lowercase hexadecimal UUID-v4 characters)",
                actual: value,
            }
            .fail();
        }

        Ok(Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for AnalysisToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
