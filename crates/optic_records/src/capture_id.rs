//! Defines the stable identity shared by all evidence from one capture.
//!
//! [`CaptureId`] is generated only after a build succeeds. Parsing and deserialization enforce the
//! same canonical textual contract as generation, so safe code cannot construct a malformed ID.
//! Durable readers also recognize the initial format-version-1 spelling and normalize it to the
//! canonical representation. Although that representation is durable, it remains opaque: callers
//! must not infer time, ordering, or storage paths from its text.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::Error;
use crate::InvalidCaptureIdSnafu;
use crate::InvalidStoredCaptureIdSnafu;
use crate::reverse_hex;

const TEXT_LENGTH: usize = 32;

/// An immutable capture identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CaptureId(String);

impl CaptureId {
    /// Creates a new random capture identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(reverse_hex::encode(uuid::Uuid::new_v4().as_bytes()))
    }

    /// Returns the opaque textual representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reads a canonical ID or the legacy spelling used by initial format-version-1 stores.
    ///
    /// This compatibility entry point is for durable record and path readers. User input must use
    /// [`str::parse`], which accepts only the canonical spelling. A legacy ID is returned in its
    /// canonical reverse-hexadecimal form.
    ///
    /// # Errors
    ///
    /// Returns an error if `value` is neither a canonical ID nor `cap_` followed by exactly 32
    /// lowercase hexadecimal characters.
    pub fn from_storage_str(value: &str) -> Result<Self, Error> {
        if let Ok(id) = value.parse() {
            return Ok(id);
        }

        let Some(suffix) = value.strip_prefix("cap_") else {
            return InvalidStoredCaptureIdSnafu {
                value: value.to_owned(),
            }
            .fail();
        };
        let valid = suffix.len() == TEXT_LENGTH
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return InvalidStoredCaptureIdSnafu {
                value: value.to_owned(),
            }
            .fail();
        }
        let uuid = uuid::Uuid::parse_str(suffix).map_err(|_| {
            InvalidStoredCaptureIdSnafu {
                value: value.to_owned(),
            }
            .build()
        })?;

        Ok(Self(reverse_hex::encode(uuid.as_bytes())))
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
        let valid = value.len() == TEXT_LENGTH && reverse_hex::decode(value).is_some();
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
    fn generates_canonical_reverse_hexadecimal_ids() {
        let id = CaptureId::new();

        assert_eq!(id.as_str().len(), 32);
        assert!(
            id.as_str()
                .bytes()
                .all(|byte| (b'k'..=b'z').contains(&byte))
        );
    }

    #[test]
    fn parses_only_full_canonical_ids() {
        let canonical = "zyxwvutsrqponmlkzyxwvutsrqponmlk";

        let parsed = canonical
            .parse::<CaptureId>()
            .expect("the canonical capture ID is valid");

        assert_eq!(parsed.as_str(), canonical);
        assert!("zyxw".parse::<CaptureId>().is_err());
        assert!(
            "zyxwvutsrqponmlkzyxwvutsrqponmlj"
                .parse::<CaptureId>()
                .is_err()
        );
        assert!(
            "cap_0123456789abcdef0123456789abcdef"
                .parse::<CaptureId>()
                .is_err()
        );
        assert!(
            serde_json::from_str::<CaptureId>(r#""cap_0123456789abcdef0123456789abcdef""#).is_err()
        );
    }

    #[test]
    fn normalizes_legacy_storage_ids() {
        let id = CaptureId::from_storage_str("cap_0123456789abcdef0123456789abcdef")
            .expect("the legacy capture ID is valid durable data");

        assert_eq!(id.as_str(), "zyxwvutsrqponmlkzyxwvutsrqponmlk");
    }
}
