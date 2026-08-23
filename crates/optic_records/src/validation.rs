use std::path::Path;

use crate::Error;
use crate::InvalidFieldSnafu;

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
