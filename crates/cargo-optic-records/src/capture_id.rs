//! Defines the stable identity shared by all evidence from one capture.
//!
//! [`CaptureId`] is generated only after a build succeeds. Parsing and deserialization enforce the
//! same canonical textual contract as generation, so safe code cannot construct a malformed ID.
//! The representation remains opaque: callers must not infer time, ordering, or storage paths from
//! its text.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;

use crate::Error;
use crate::error::InvalidCaptureIdSnafu;
use crate::reverse_hex;

const TEXT_LENGTH: usize = 32;

/// An immutable identifier for all evidence from one capture.
///
/// Capture IDs use the reverse-hexadecimal alphabet `k` through `z`. This alphabet makes the text
/// distinct from future instance IDs at boundaries that cannot use the Rust type.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CaptureId(String);

impl CaptureId {
    /// Generates a new random capture identifier.
    pub fn generate() -> Self {
        Self(reverse_hex::encode(uuid::Uuid::new_v4().as_bytes()))
    }

    /// Returns the opaque textual representation.
    pub fn as_str(&self) -> &str {
        &self.0
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
        let valid = value.len() == TEXT_LENGTH && reverse_hex::is_canonical(value);
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
    use crate::Error;

    #[test]
    fn generates_canonical_reverse_hexadecimal_ids() {
        let id = CaptureId::generate();

        assert_eq!(id.as_str().len(), 32);
        assert!(
            id.as_str()
                .bytes()
                .all(|byte| (b'k'..=b'z').contains(&byte))
        );
    }

    #[test]
    fn parses_a_full_canonical_id() {
        let canonical = "zyxwvutsrqponmlkzyxwvutsrqponmlk";
        let parsed = canonical
            .parse::<CaptureId>()
            .expect("the canonical capture ID is valid");

        assert_eq!(parsed.as_str(), canonical);
    }

    #[test]
    fn rejects_each_noncanonical_text_shape() {
        for value in [
            "zyxw",                                 // Too short.
            "zyxwvutsrqponmlkzyxwvutsrqponmlj",     // Digit outside `k` through `z`.
            "cap_0123456789abcdef0123456789abcdef", // Different identifier grammar.
        ] {
            let error = value
                .parse::<CaptureId>()
                .expect_err("the noncanonical capture ID must be rejected");

            assert!(matches!(
                error,
                Error::InvalidCaptureId { value: actual } if actual == value
            ));
        }
    }

    #[test]
    fn deserialization_uses_the_same_canonical_text_contract() {
        let error = serde_json::from_str::<CaptureId>(r#""cap_0123456789abcdef0123456789abcdef""#)
            .expect_err("the noncanonical capture ID must be rejected");

        assert!(
            error
                .to_string()
                .contains("capture ID must contain exactly 32")
        );
    }
}
