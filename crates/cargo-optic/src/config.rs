//! Reads the workspace-local Cargo Optic configuration.
//!
//! The prototype accepts one storage limit below `.optic/config.toml`. Command-specific limits
//! override this file, while an adaptive default limits an unconfigured store. The policy keeps
//! the retained-byte limit separate from the available-space reserve because both constraints
//! must hold before a capture can add evidence.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result};

const DEFAULT_MAXIMUM_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const AVAILABLE_SPACE_RESERVE: u64 = 10 * 1024 * 1024 * 1024;

/// The effective limits for retained compiler evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StorePolicy {
    /// The maximum number of bytes retained below `.optic/store`.
    pub(crate) maximum_bytes: u64,

    /// The filesystem space that a capture must leave available.
    pub(crate) available_space_reserve: u64,
}

/// Reads `.optic/config.toml` and resolves the effective storage policy.
///
/// A command-specific limit takes precedence over the configured limit. When neither is present,
/// the retained-byte limit is the smaller of 32 GiB and one quarter of filesystem capacity. The
/// available-space reserve is the smaller of 10 GiB and five percent of filesystem capacity.
pub(crate) fn load_store_policy(
    optic_directory: &Path,
    command_maximum_bytes: Option<u64>,
    filesystem_bytes: u64,
) -> Result<StorePolicy> {
    let configured_maximum_bytes = read_configuration(optic_directory)?.store.max_bytes;

    Ok(resolve_store_policy(
        command_maximum_bytes,
        configured_maximum_bytes,
        filesystem_bytes,
    ))
}

/// Parses an unsigned byte count with an optional binary unit.
pub(crate) fn parse_byte_size(value: &str) -> Result<u64> {
    parse_byte_size_value(value).map_err(|message| Error::InvalidRequest { message })
}

fn resolve_store_policy(
    command_maximum_bytes: Option<u64>,
    configured_maximum_bytes: Option<u64>,
    filesystem_bytes: u64,
) -> StorePolicy {
    let default_maximum_bytes = DEFAULT_MAXIMUM_BYTES.min(filesystem_bytes / 4);
    let maximum_bytes = command_maximum_bytes
        .or(configured_maximum_bytes)
        .unwrap_or(default_maximum_bytes);

    StorePolicy {
        maximum_bytes,
        available_space_reserve: AVAILABLE_SPACE_RESERVE.min(filesystem_bytes / 20),
    }
}

fn read_configuration(optic_directory: &Path) -> Result<OpticConfiguration> {
    let path = optic_directory.join("config.toml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OpticConfiguration::default());
        }
        Err(source) => return Err(Error::filesystem("read", &path, source)),
    };

    toml::from_str(&text).map_err(|source| Error::InvalidRequest {
        message: format!(
            "Optic configuration must be valid at {}, got {source}",
            path.display()
        ),
    })
}

fn parse_byte_size_value(value: &str) -> std::result::Result<u64, String> {
    let (digits, multiplier) = if let Some(digits) = value.strip_suffix("TiB") {
        (digits, 1024_u64.pow(4))
    } else if let Some(digits) = value.strip_suffix("GiB") {
        (digits, 1024_u64.pow(3))
    } else if let Some(digits) = value.strip_suffix("MiB") {
        (digits, 1024_u64.pow(2))
    } else if let Some(digits) = value.strip_suffix("KiB") {
        (digits, 1024)
    } else if let Some(digits) = value.strip_suffix('B') {
        (digits, 1)
    } else {
        (value, 1)
    };
    let bytes = digits.parse::<u64>().map_err(|_| {
        format!(
            "byte size must be an unsigned integer with an optional B, KiB, MiB, GiB, or TiB suffix, got {value}"
        )
    })?;

    bytes
        .checked_mul(multiplier)
        .ok_or_else(|| format!("byte size must be at most {} bytes, got {value}", u64::MAX))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpticConfiguration {
    #[serde(default)]
    store: StoreConfiguration,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreConfiguration {
    #[serde(default, deserialize_with = "deserialize_optional_byte_size")]
    max_bytes: Option<u64>,
}

fn deserialize_optional_byte_size<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;

    value
        .map(|value| parse_byte_size_value(&value).map_err(serde::de::Error::custom))
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        AVAILABLE_SPACE_RESERVE, DEFAULT_MAXIMUM_BYTES, StorePolicy, load_store_policy,
        parse_byte_size, resolve_store_policy,
    };

    #[test]
    fn parses_supported_byte_sizes() {
        for (value, expected) in [
            ("0", 0),                         //
            ("1B", 1),                        //
            ("2KiB", 2 * 1024),               //
            ("3MiB", 3 * 1024 * 1024),        //
            ("4GiB", 4 * 1024 * 1024 * 1024), //
            ("5TiB", 5 * 1024_u64.pow(4)),    //
        ] {
            assert_eq!(
                parse_byte_size(value).expect("the byte size is valid"),
                expected
            );
        }
    }

    #[test]
    fn rejects_invalid_and_overflowing_byte_sizes() {
        for value in ["", "-1", "1.5GiB", "1GB", "18446744073709551615TiB"] {
            assert!(parse_byte_size(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn resolves_command_configuration_and_default_precedence() {
        let filesystem_bytes = 256 * 1024_u64.pow(3);

        assert_eq!(
            resolve_store_policy(Some(3), Some(2), filesystem_bytes),
            policy(3)
        );
        assert_eq!(
            resolve_store_policy(None, Some(2), filesystem_bytes),
            policy(2)
        );
        assert_eq!(
            resolve_store_policy(None, None, filesystem_bytes),
            policy(DEFAULT_MAXIMUM_BYTES)
        );
    }

    #[test]
    fn limits_the_default_to_one_quarter_of_the_filesystem() {
        let filesystem_bytes = 16 * 1024_u64.pow(3);

        assert_eq!(
            resolve_store_policy(None, None, filesystem_bytes),
            StorePolicy {
                maximum_bytes: 4 * 1024_u64.pow(3),
                available_space_reserve: filesystem_bytes / 20,
            }
        );
    }

    #[test]
    fn reads_the_optional_workspace_configuration() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let optic = temporary.path().join(".optic");
        fs::create_dir(&optic).expect("the test can create the Optic directory");
        fs::write(
            optic.join("config.toml"),
            "[store]\nmax_bytes = \"64GiB\"\n",
        )
        .expect("the test can write the Optic configuration");

        let configured = load_store_policy(&optic, None, 512 * 1024_u64.pow(3))
            .expect("the Optic configuration is valid");
        let overridden = load_store_policy(&optic, Some(7), 512 * 1024_u64.pow(3))
            .expect("the command override is valid");

        assert_eq!(configured, policy(64 * 1024_u64.pow(3)));
        assert_eq!(overridden, policy(7));
    }

    #[test]
    fn uses_the_default_when_configuration_is_absent() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");

        assert_eq!(
            load_store_policy(temporary.path(), None, 512 * 1024_u64.pow(3))
                .expect("an absent configuration uses defaults"),
            policy(DEFAULT_MAXIMUM_BYTES)
        );
    }

    #[test]
    fn rejects_unknown_configuration_fields() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("config.toml");
        fs::write(&path, "[store]\nmaximum_bytes = \"1GiB\"\n")
            .expect("the test can write an invalid Optic configuration");

        let error = load_store_policy(temporary.path(), None, 512 * 1024_u64.pow(3))
            .expect_err("unknown fields are rejected");

        assert!(error.to_string().contains("unknown field `maximum_bytes`"));
    }

    fn policy(maximum_bytes: u64) -> StorePolicy {
        StorePolicy {
            maximum_bytes,
            available_space_reserve: AVAILABLE_SPACE_RESERVE,
        }
    }
}
