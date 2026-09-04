//! Publishes complete captures through one atomic namespace change.
//!
//! [`Store::publish`] writes and flushes the capture record and instance manifest before renaming
//! their staging directory into the completed namespace. The rename is the atomic visibility
//! boundary.

use std::fs;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;

use optic_records::CaptureRecord;
use optic_records::InstanceManifest;
use snafu::ResultExt;

use crate::CAPTURE_FILE_NAME;
use crate::Error;
use crate::INSTANCES_FILE_NAME;
use crate::Store;
use crate::error::CaptureExistsSnafu;
use crate::error::FilesystemSnafu;
use crate::error::JsonSnafu;
use crate::error::MismatchedInstanceCountsSnafu;
use crate::error::MismatchedPublishedCaptureIdSnafu;

impl Store {
    /// Atomically publishes one complete immutable capture and its instance evidence.
    ///
    /// # Errors
    ///
    /// Returns an error if the records disagree or publication fails before the final rename. A
    /// failed publication is not visible through [`Self::list_captures`].
    pub fn publish(
        &self,
        capture: &CaptureRecord,
        instances: &InstanceManifest,
    ) -> Result<(), Error> {
        if capture.id() != instances.capture_id() {
            return MismatchedPublishedCaptureIdSnafu {
                capture_id: capture.id().clone(),
                manifest_id: instances.capture_id().clone(),
            }
            .fail();
        }
        validate_instance_counts(capture, instances)?;

        let staging_root = self.root.join("staging");
        let captures_root = self.root.join("captures");

        fs::create_dir_all(&staging_root).with_context(|_| FilesystemSnafu {
            operation: "create",
            path: staging_root.clone(),
        })?;
        fs::create_dir_all(&captures_root).with_context(|_| FilesystemSnafu {
            operation: "create",
            path: captures_root.clone(),
        })?;

        let staging = staging_root.join(capture.id().as_str());
        let completed = captures_root.join(capture.id().as_str());
        if completed.exists() {
            return CaptureExistsSnafu {
                id: capture.id().clone(),
            }
            .fail();
        }

        fs::create_dir(&staging).with_context(|_| FilesystemSnafu {
            operation: "create",
            path: staging.clone(),
        })?;

        if let Err(error) = write_capture(&staging.join(CAPTURE_FILE_NAME), capture) {
            let _ = fs::remove_dir_all(&staging);

            return Err(error);
        }

        if let Err(error) = write_instances(&staging.join(INSTANCES_FILE_NAME), instances) {
            let _ = fs::remove_dir_all(&staging);

            return Err(error);
        }

        // TODO(connor)[Crash durability]: Synchronize both record files, the staged capture
        // directory, and both namespace directories around the rename if Cargo Optic guarantees
        // persistence after a system crash. This version guarantees atomic visibility only, so it
        // does not add platform-specific synchronization or a post-commit durability warning.
        if let Err(error) = fs::rename(&staging, &completed).with_context(|_| FilesystemSnafu {
            operation: "publish",
            path: completed,
        }) {
            let _ = fs::remove_dir_all(&staging);

            return Err(error);
        }

        Ok(())
    }
}

pub(super) fn validate_instance_counts(
    capture: &CaptureRecord,
    instances: &InstanceManifest,
) -> Result<(), Error> {
    let manifest_instance_count = instances.instance_count();
    let manifest_placement_count = instances.placement_count();
    if capture.instance_count() != manifest_instance_count
        || capture.placement_count() != manifest_placement_count
    {
        return MismatchedInstanceCountsSnafu {
            capture_id: capture.id().clone(),
            capture_instance_count: capture.instance_count(),
            capture_placement_count: capture.placement_count(),
            manifest_instance_count,
            manifest_placement_count,
        }
        .fail();
    }

    Ok(())
}

fn write_capture(path: &Path, capture: &CaptureRecord) -> Result<(), Error> {
    write_record(path, |writer| serde_json::to_writer_pretty(writer, capture))
}

fn write_instances(path: &Path, instances: &InstanceManifest) -> Result<(), Error> {
    write_record(path, |writer| serde_json::to_writer(writer, instances))
}

fn write_record(
    path: &Path,
    encode: impl FnOnce(&mut BufWriter<fs::File>) -> serde_json::Result<()>,
) -> Result<(), Error> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|_| FilesystemSnafu {
            operation: "create",
            path: path.to_owned(),
        })?;

    let mut writer = BufWriter::new(file);

    encode(&mut writer).with_context(|_| JsonSnafu {
        path: path.to_owned(),
    })?;
    writer.write_all(b"\n").with_context(|_| FilesystemSnafu {
        operation: "write",
        path: path.to_owned(),
    })?;

    writer.flush().with_context(|_| FilesystemSnafu {
        operation: "write",
        path: path.to_owned(),
    })
}
