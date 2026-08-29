//! Keeps shared record-field checks consistent across construction and deserialization.
//!
//! Each record retains its field-specific invariants so these helpers do not become a general
//! validation layer.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::Error;
use crate::error::InvalidFieldSnafu;

pub(crate) fn require_text(field: &'static str, value: &str) -> Result<(), Error> {
    if value.is_empty() {
        return InvalidFieldSnafu {
            field,
            actual: "an empty string",
        }
        .fail();
    }

    Ok(())
}

pub(crate) fn require_path(field: &'static str, value: &Path) -> Result<(), Error> {
    if value.as_os_str().is_empty() {
        return InvalidFieldSnafu {
            field,
            actual: "an empty path",
        }
        .fail();
    }

    Ok(())
}

pub(crate) fn require_absolute_normalized_path(
    field: &'static str,
    value: &Path,
) -> Result<(), Error> {
    require_path(field, value)?;

    if !value.is_absolute() {
        return InvalidFieldSnafu {
            field,
            actual: format!("a relative path ({})", value.display()),
        }
        .fail();
    }

    let normalized = value.components().collect::<PathBuf>();
    let has_parent = value
        .components()
        .any(|component| matches!(component, Component::ParentDir));
    if has_parent || normalized.as_os_str() != value.as_os_str() {
        return InvalidFieldSnafu {
            field,
            actual: format!(
                "a path that is not lexically normalized ({})",
                value.display()
            ),
        }
        .fail();
    }

    Ok(())
}
