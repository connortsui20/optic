//! Publishes complete capture records through one atomic namespace change.
//!
//! [`Store::publish`] writes and flushes the record before renaming its staging directory into the
//! completed namespace. The rename is the atomic visibility boundary.

use std::fs;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;

use optic_records::CaptureRecord;
use snafu::ResultExt;

use crate::Error;
use crate::RECORD_FILE_NAME;
use crate::Store;
use crate::error::CaptureExistsSnafu;
use crate::error::FilesystemSnafu;
use crate::error::JsonSnafu;

impl Store {
    /// Atomically publishes one complete immutable capture.
    ///
    /// # Errors
    ///
    /// Returns an error if publication fails before the final rename. A failed publication is not
    /// visible through [`Self::list_captures`].
    pub fn publish(&self, capture: &CaptureRecord) -> Result<(), Error> {
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

        if let Err(error) = write_record(&staging.join(RECORD_FILE_NAME), capture) {
            let _ = fs::remove_dir_all(&staging);

            return Err(error);
        }

        // TODO(connor)[Crash durability]: Synchronize the record file, the staged capture
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

fn write_record(path: &Path, capture: &CaptureRecord) -> Result<(), Error> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|_| FilesystemSnafu {
            operation: "create",
            path: path.to_owned(),
        })?;

    let mut writer = BufWriter::new(file);

    serde_json::to_writer_pretty(&mut writer, capture).with_context(|_| JsonSnafu {
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
