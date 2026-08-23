use std::fs;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;

use optic_records::CaptureRecord;
use snafu::ResultExt;

use crate::CaptureExistsSnafu;
use crate::Error;
use crate::FilesystemSnafu;
use crate::JsonSnafu;
use crate::RECORD_FILE_NAME;
use crate::Store;

impl Store {
    /// Atomically publishes one complete immutable capture.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture ID already exists or a filesystem or JSON operation fails.
    /// A failed publication is never visible through [`Self::captures`].
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

        let result = write_record(&staging.join(RECORD_FILE_NAME), capture).and_then(|()| {
            fs::rename(&staging, &completed).with_context(|_| FilesystemSnafu {
                operation: "publish",
                path: completed.clone(),
            })
        });
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }

        result
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
    })?;
    writer
        .get_ref()
        .sync_all()
        .with_context(|_| FilesystemSnafu {
            operation: "synchronize",
            path: path.to_owned(),
        })
}
