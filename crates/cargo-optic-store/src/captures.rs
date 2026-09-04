//! Reads completed captures without trusting the store contents.
//!
//! Capture history reads validate the small capture header and require its instance manifest file.
//! Evidence reads additionally deserialize the full manifest and validate that it agrees with the
//! header. This keeps listing cost proportional to capture history rather than evidence size.

use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::InstanceManifest;
use serde::de::DeserializeOwned;
use snafu::IntoError;
use snafu::ResultExt;

use crate::CAPTURE_FILE_NAME;
use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::Store;
use crate::error::CaptureNotFoundSnafu;
use crate::error::ExpectedCaptureDirectorySnafu;
use crate::error::ExpectedInstanceFileSnafu;
use crate::error::FilesystemSnafu;
use crate::error::InvalidCaptureDirectoryIdSnafu;
use crate::error::InvalidCaptureDirectoryNameSnafu;
use crate::error::JsonSnafu;
use crate::error::MismatchedCaptureIdSnafu;
use crate::publish::validate_instance_counts;

impl Store {
    /// Lists captures by descending recorded completion time, then ascending capture ID.
    ///
    /// # Errors
    ///
    /// Returns an error if any completed entry or capture record is invalid, or if its instance
    /// manifest is not present as a file.
    pub fn list_captures(&self) -> Result<Vec<CaptureRecord>, Error> {
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
            let directory_id = capture_id_from_entry(&entry)?;
            let directory = entry.path();
            let capture = read_capture_from_directory(&directory, &directory_id)?;
            require_instances_file(&directory)?;

            captures.push(capture);
        }

        captures.sort_by(|left, right| {
            right
                .completed_at_unix_ms()
                .cmp(&left.completed_at_unix_ms())
                .then_with(|| left.id().as_str().cmp(right.id().as_str()))
        });

        Ok(captures)
    }

    /// Reads one capture record from the completed namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture record is invalid or scoped to a different capture, or if
    /// its instance manifest is not present as a file.
    pub fn read_capture(&self, id: &CaptureId) -> Result<CaptureRecord, Error> {
        let directory = self.capture_directory(id)?;
        let capture = read_capture_from_directory(&directory, id)?;
        require_instances_file(&directory)?;

        Ok(capture)
    }

    /// Reads one instance manifest from the completed namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if the instance manifest or its capture record is missing, invalid, scoped
    /// to a different capture, or contains evidence counts that disagree with its capture header.
    pub fn read_instances(&self, id: &CaptureId) -> Result<InstanceManifest, Error> {
        let directory = self.capture_directory(id)?;
        let capture = read_capture_from_directory(&directory, id)?;
        let instances_path = require_instances_file(&directory)?;
        let instances: InstanceManifest = read_record(&instances_path)?;
        validate_capture_scope(&instances_path, id, instances.capture_id())?;
        validate_instance_counts(&capture, &instances)?;

        Ok(instances)
    }

    fn capture_directory(&self, id: &CaptureId) -> Result<PathBuf, Error> {
        let directory = self.root.join("captures").join(id.as_str());
        match fs::metadata(&directory) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return ExpectedCaptureDirectorySnafu { path: directory }.fail(),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return CaptureNotFoundSnafu { id: id.clone() }.fail();
            }
            Err(source) => {
                return Err(FilesystemSnafu {
                    operation: "read metadata for",
                    path: directory,
                }
                .into_error(source));
            }
        }

        Ok(directory)
    }
}

fn read_capture_from_directory(
    directory: &Path,
    directory_id: &CaptureId,
) -> Result<CaptureRecord, Error> {
    let capture_path = directory.join(CAPTURE_FILE_NAME);
    let capture: CaptureRecord = read_record(&capture_path)?;
    validate_capture_scope(&capture_path, directory_id, capture.id())?;

    Ok(capture)
}

fn require_instances_file(directory: &Path) -> Result<PathBuf, Error> {
    let path = directory.join(INSTANCES_FILE_NAME);
    let metadata = fs::metadata(&path).with_context(|_| FilesystemSnafu {
        operation: "read metadata for",
        path: path.clone(),
    })?;
    if !metadata.is_file() {
        return ExpectedInstanceFileSnafu { path }.fail();
    }

    Ok(path)
}

fn capture_id_from_entry(entry: &fs::DirEntry) -> Result<CaptureId, Error> {
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

    name.parse::<CaptureId>()
        .with_context(|_| InvalidCaptureDirectoryIdSnafu {
            path: entry_path,
            name,
        })
}

fn read_record<T: DeserializeOwned>(path: &Path) -> Result<T, Error> {
    let reader = BufReader::new(File::open(path).with_context(|_| FilesystemSnafu {
        operation: "open",
        path: path.to_owned(),
    })?);

    serde_json::from_reader(reader).with_context(|_| JsonSnafu {
        path: path.to_owned(),
    })
}

fn validate_capture_scope(
    path: &Path,
    directory_id: &CaptureId,
    record_id: &CaptureId,
) -> Result<(), Error> {
    if record_id != directory_id {
        return MismatchedCaptureIdSnafu {
            path: path.to_owned(),
            directory_id: directory_id.clone(),
            record_id: record_id.clone(),
        }
        .fail();
    }

    Ok(())
}
