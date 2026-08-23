//! Defines the stable identity shared by all evidence from one capture.
//!
//! [`CaptureId`] is generated only after a build succeeds. Parsing and deserialization enforce the
//! same textual contract as generation, so safe code cannot construct a malformed ID. Although the
//! representation is durable, it remains opaque: callers must not infer time, ordering, or storage
//! paths from its text.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::Error;
use crate::InvalidCaptureIdSnafu;

/// An immutable capture identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CaptureId(String);

impl CaptureId {
    /// Creates a new random capture identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("cap_{}", uuid::Uuid::new_v4().simple()))
    }

    /// Returns the opaque textual representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for CaptureId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CaptureId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for CaptureId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(suffix) = value.strip_prefix("cap_") else {
            return InvalidCaptureIdSnafu {
                value: value.to_owned(),
            }
            .fail();
        };
        let valid = suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return InvalidCaptureIdSnafu {
                value: value.to_owned(),
            }
            .fail();
        }

        Ok(Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for CaptureId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(<D::Error as serde::de::Error>::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::CaptureId;

    #[test]
    fn rejects_invalid_parsed_and_deserialized_text() {
        assert!("capture_1234".parse::<CaptureId>().is_err());
        assert!(serde_json::from_str::<CaptureId>(r#""capture_1234""#).is_err());
    }
}
