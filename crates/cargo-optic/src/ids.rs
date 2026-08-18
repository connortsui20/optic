//! Opaque identifiers used by public Optic commands.
//!
//! IDs do not expose SQLite row identifiers or artifact paths. Their prefixes prevent callers from
//! accidentally using a capture ID where an instance ID is required.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

macro_rules! identifier {
    ($name:ident, $prefix:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub(crate) fn new() -> Self {
                Self(format!(concat!($prefix, "_{}"), Uuid::now_v7().simple()))
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
                let Some(uuid) = value.strip_prefix(concat!($prefix, "_")) else {
                    return Err(Error::InvalidRequest {
                        message: format!(
                            concat!("expected an ", $prefix, " identifier, got {}"),
                            value
                        ),
                    });
                };
                Uuid::parse_str(uuid).map_err(|_| Error::InvalidRequest {
                    message: format!(
                        concat!("expected an ", $prefix, " identifier, got {}"),
                        value
                    ),
                })?;

                Ok(Self(value.to_owned()))
            }
        }
    };
}

identifier!(CaptureId, "cap", "An immutable completed capture.");
identifier!(InstanceId, "ins", "One concrete Rust compiler instance.");
