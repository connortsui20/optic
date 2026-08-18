//! Opaque identifiers used by public Optic commands.
//!
//! IDs do not expose SQLite row identifiers or artifact paths. Their prefixes prevent callers from
//! accidentally using a capture ID where an instance ID is required.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

macro_rules! identifier {
    ($name:ident, $prefix:literal, $kind:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub(crate) fn new() -> Self {
                Self(format!(concat!($prefix, "_{}"), Uuid::new_v4().simple()))
            }

            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                let Some(suffix) = value.strip_prefix(concat!($prefix, "_")) else {
                    return Err(Error::InvalidRequest {
                        message: format!(
                            concat!($kind, " ID must start with `", $prefix, "_`, got {}"),
                            value
                        ),
                    });
                };
                let valid_length = (1..=32).contains(&suffix.len());
                let valid_characters = suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));

                if !valid_length || !valid_characters {
                    return Err(Error::InvalidRequest {
                        message: format!(
                            concat!(
                                $kind,
                                " ID must contain 1 to 32 lowercase hexadecimal characters, got {}"
                            ),
                            value
                        ),
                    });
                }

                Ok(Self(value.to_owned()))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;

                value.parse().map_err(D::Error::custom)
            }
        }
    };
}

identifier!(
    CaptureId,
    "cap",
    "capture",
    "An immutable capture ID or its unique prefix."
);
identifier!(
    InstanceId,
    "ins",
    "instance",
    "A concrete compiler-instance ID or its unique prefix."
);

#[cfg(test)]
mod tests {
    use super::{CaptureId, InstanceId};

    #[test]
    fn accepts_full_ids_and_short_prefixes() {
        assert!("cap_0123".parse::<CaptureId>().is_ok());
        assert!(
            "ins_0123456789abcdef0123456789abcdef"
                .parse::<InstanceId>()
                .is_ok()
        );
    }

    #[test]
    fn rejects_empty_long_and_non_hexadecimal_suffixes() {
        assert!("cap_".parse::<CaptureId>().is_err());
        assert!(
            "cap_0123456789abcdef0123456789abcdef0"
                .parse::<CaptureId>()
                .is_err()
        );
        assert!("ins_01xz".parse::<InstanceId>().is_err());
    }
}
