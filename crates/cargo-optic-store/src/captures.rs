use std::fs;
use std::fs::File;
use std::io::BufReader;

use optic_records::CaptureId;
use optic_records::CaptureRecord;
use snafu::IntoError;
use snafu::ResultExt;

use crate::Error;
use crate::RECORD_FILE_NAME;
use crate::Store;
use crate::error::ExpectedCaptureDirectorySnafu;
use crate::error::FilesystemSnafu;
use crate::error::InvalidCaptureDirectoryIdSnafu;
use crate::error::InvalidCaptureDirectoryNameSnafu;
use crate::error::JsonSnafu;
use crate::error::MismatchedCaptureIdSnafu;

impl Store {
    /// Lists captures by descending recorded completion time, then ascending capture ID.
    ///
    /// # Errors
    ///
    /// Returns an error if any completed entry or record is invalid or cannot be read.
    pub fn captures(&self) -> Result<Vec<CaptureRecord>, Error> {
        let captures_root = self.root.join("captures");
        let entries = match fs::read_dir(&captures_root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(FilesystemSnafu {
                    operation: "read",
                    path: captures_root,
                }
                .into_error(source));
            }
        };
        let mut captures = Vec::new();

        for entry in entries {
            let entry = entry.with_context(|_| FilesystemSnafu {
                operation: "read",
                path: captures_root.clone(),
            })?;

            captures.push(read_capture(entry)?);
        }

        captures.sort_by(|left, right| {
            right
                .completed_at_unix_ms()
                .cmp(&left.completed_at_unix_ms())
                .then_with(|| left.id().as_str().cmp(right.id().as_str()))
        });

        Ok(captures)
    }
}

fn read_capture(entry: fs::DirEntry) -> Result<CaptureRecord, Error> {
    let entry_path = entry.path();
    let file_type = entry.file_type().with_context(|_| FilesystemSnafu {
        operation: "read metadata for",
        path: entry_path.clone(),
    })?;
    if !file_type.is_dir() {
        return ExpectedCaptureDirectorySnafu { path: entry_path }.fail();
    }

    let name = entry.file_name().into_string().map_err(|name| {
        InvalidCaptureDirectoryNameSnafu {
            path: entry_path.clone(),
            name,
        }
        .build()
    })?;
    let directory_id =
        name.parse::<CaptureId>()
            .with_context(|_| InvalidCaptureDirectoryIdSnafu {
                path: entry_path.clone(),
                name: name.clone(),
            })?;
    let path = entry_path.join(RECORD_FILE_NAME);
    let reader = BufReader::new(File::open(&path).with_context(|_| FilesystemSnafu {
        operation: "open",
        path: path.clone(),
    })?);
    let capture = serde_json::from_reader::<_, CaptureRecord>(reader)
        .with_context(|_| JsonSnafu { path: path.clone() })?;
    if capture.id() != &directory_id {
        return MismatchedCaptureIdSnafu {
            path,
            directory_id,
            record_id: capture.id().clone(),
        }
        .fail();
    }

    Ok(capture)
}
