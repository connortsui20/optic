//! Copies declared artifacts and streams finite ranges from completed captures.
//!
//! Generated filenames stay directly inside the capture directory. Metadata checks reject symlinks,
//! wrong file types, and length changes. Range reads never open the current source checkout.

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;

use optic_records::ArtifactId;
use optic_records::ArtifactRecord;
use optic_records::ByteRange;
use optic_records::CaptureId;
use optic_records::InstanceManifest;
use snafu::ResultExt;

use crate::Error;
use crate::Store;
use crate::error::ArtifactLengthSnafu;
use crate::error::ArtifactRangeSnafu;
use crate::error::FilesystemSnafu;
use crate::error::UnexpectedFileTypeSnafu;
use crate::error::UnknownArtifactSnafu;
use crate::error::WriteEvidenceSnafu;

// Artifact copying uses a fixed 64 KiB buffer independent of file length and requested range.
const COPY_BUFFER_BYTES: usize = 64 * 1024;

impl Store {
    /// Copies a finite range from one declared artifact in a completed capture.
    ///
    /// Empty ranges are valid, including a range at EOF. All declared artifacts must pass metadata
    /// validation. The caller must prevent external store mutation throughout the operation.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid capture or artifact identity, corrupt metadata, invalid bounds,
    /// premature EOF, or I/O failure. Writer failures use [`Error::WriteEvidence`]. Partial output
    /// is possible before a read or write error.
    pub fn copy_evidence(
        &self,
        capture: &CaptureId,
        artifact: ArtifactId,
        range: ByteRange,
        writer: &mut impl Write,
    ) -> Result<(), Error> {
        let manifest = self.read_instances(capture)?;
        let record = manifest
            .artifacts()
            .iter()
            .find(|record| record.id() == artifact)
            .ok_or_else(|| {
                UnknownArtifactSnafu {
                    capture: capture.clone(),
                    artifact,
                }
                .build()
            })?;
        if range.end() > record.byte_len() {
            return ArtifactRangeSnafu {
                artifact,
                start: range.start(),
                length: range.length(),
                byte_len: record.byte_len(),
            }
            .fail();
        }

        let path = self.capture_directory(capture)?.join(record.file_name());
        let mut file = File::open(&path).with_context(|_| FilesystemSnafu {
            operation: "open artifact",
            path: path.clone(),
        })?;
        file.seek(SeekFrom::Start(range.start()))
            .with_context(|_| FilesystemSnafu {
                operation: "seek artifact",
                path: path.clone(),
            })?;

        copy_evidence_bytes(&mut file, range.length(), writer, &path)
    }
}

pub(crate) fn validate_artifacts(
    directory: &Path,
    manifest: &InstanceManifest,
) -> Result<(), Error> {
    for artifact in manifest.artifacts() {
        validate_artifact_file(&directory.join(artifact.file_name()), artifact)?;
    }

    Ok(())
}

pub(crate) fn copy_artifacts(
    input_directory: &Path,
    staging: &Path,
    manifest: &InstanceManifest,
) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(input_directory).with_context(|_| FilesystemSnafu {
        operation: "read artifact directory",
        path: input_directory.to_owned(),
    })?;
    if !metadata.is_dir() {
        return UnexpectedFileTypeSnafu {
            expected: "directory",
            path: input_directory.to_owned(),
        }
        .fail();
    }

    let mut buffer = [0; COPY_BUFFER_BYTES];
    for artifact in manifest.artifacts() {
        let source = input_directory.join(artifact.file_name());
        let destination = staging.join(artifact.file_name());
        validate_artifact_file(&source, artifact)?;
        let mut reader = File::open(&source).with_context(|_| FilesystemSnafu {
            operation: "open artifact",
            path: source.clone(),
        })?;
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .with_context(|_| FilesystemSnafu {
                operation: "create artifact",
                path: destination.clone(),
            })?;
        let mut remaining = artifact.byte_len();
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            reader
                .read_exact(&mut buffer[..count])
                .with_context(|_| FilesystemSnafu {
                    operation: "read artifact",
                    path: source.clone(),
                })?;
            writer
                .write_all(&buffer[..count])
                .with_context(|_| FilesystemSnafu {
                    operation: "write artifact",
                    path: destination.clone(),
                })?;
            remaining -= count as u64;
        }
        validate_artifact_file(&source, artifact)?;
    }

    validate_artifacts(staging, manifest)
}

fn validate_artifact_file(path: &Path, artifact: &ArtifactRecord) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path).with_context(|_| FilesystemSnafu {
        operation: "read artifact metadata",
        path: path.to_owned(),
    })?;
    if !metadata.is_file() {
        return UnexpectedFileTypeSnafu {
            expected: "file",
            path: path.to_owned(),
        }
        .fail();
    }
    if metadata.len() != artifact.byte_len() {
        return ArtifactLengthSnafu {
            path: path.to_owned(),
            expected: artifact.byte_len(),
            actual: metadata.len(),
        }
        .fail();
    }

    Ok(())
}

/// Separates reader and caller-writer errors, including EOF after a successful metadata check.
pub(crate) fn copy_evidence_bytes(
    reader: &mut impl Read,
    length: u64,
    writer: &mut impl Write,
    path: &Path,
) -> Result<(), Error> {
    let mut buffer = [0; COPY_BUFFER_BYTES];
    let mut remaining = length;
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        reader
            .read_exact(&mut buffer[..count])
            .with_context(|_| FilesystemSnafu {
                operation: "read artifact",
                path: path.to_owned(),
            })?;
        writer
            .write_all(&buffer[..count])
            .context(WriteEvidenceSnafu)?;
        remaining -= count as u64;
    }

    Ok(())
}
