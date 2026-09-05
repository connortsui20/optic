//! Applies the same encoded-size budgets to durable readers and writers.
//!
//! Readers check metadata before allocation and cap the actual read before deserialization.
//! Writers stop serialization at the same boundary, so oversized records cannot reach publication.

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use snafu::ResultExt;

use crate::Error;
use crate::error::FilesystemSnafu;
use crate::error::JsonSnafu;
use crate::error::RecordTooLargeSnafu;
use crate::error::UnexpectedFileTypeSnafu;

pub(crate) fn read_record<T: DeserializeOwned>(path: &Path, limit: u64) -> Result<T, Error> {
    let metadata = fs::symlink_metadata(path).with_context(|_| FilesystemSnafu {
        operation: "open",
        path: path.to_owned(),
    })?;
    if !metadata.is_file() {
        return UnexpectedFileTypeSnafu {
            expected: "file",
            path: path.to_owned(),
        }
        .fail();
    }
    if metadata.len() > limit {
        return RecordTooLargeSnafu {
            path: path.to_owned(),
            limit,
            actual: metadata.len(),
        }
        .fail();
    }

    let file = File::open(path).with_context(|_| FilesystemSnafu {
        operation: "open",
        path: path.to_owned(),
    })?;
    read_bounded_record(file, path, limit)
}

// The separate reader boundary also covers files that grow after the metadata check.
pub(crate) fn read_bounded_record<T: DeserializeOwned>(
    reader: impl Read,
    path: &Path,
    limit: u64,
) -> Result<T, Error> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|_| FilesystemSnafu {
            operation: "read",
            path: path.to_owned(),
        })?;
    if bytes.len() as u64 > limit {
        return RecordTooLargeSnafu {
            path: path.to_owned(),
            limit,
            actual: bytes.len() as u64,
        }
        .fail();
    }

    serde_json::from_slice(&bytes).with_context(|_| JsonSnafu {
        path: path.to_owned(),
    })
}

pub(crate) fn write_record<T: Serialize>(path: &Path, record: &T, limit: u64) -> Result<(), Error> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|_| FilesystemSnafu {
            operation: "create",
            path: path.to_owned(),
        })?;
    let mut writer = BoundedWriter {
        inner: BufWriter::new(file),
        remaining: limit,
        exceeded: false,
    };
    let encoded = serde_json::to_writer(&mut writer, record);
    if writer.exceeded {
        return RecordTooLargeSnafu {
            path: path.to_owned(),
            limit,
            actual: limit + 1,
        }
        .fail();
    }
    encoded.with_context(|_| JsonSnafu {
        path: path.to_owned(),
    })?;
    if writer.remaining == 0 {
        return RecordTooLargeSnafu {
            path: path.to_owned(),
            limit,
            actual: limit + 1,
        }
        .fail();
    }
    writer.write_all(b"\n").with_context(|_| FilesystemSnafu {
        operation: "write",
        path: path.to_owned(),
    })?;

    writer.flush().with_context(|_| FilesystemSnafu {
        operation: "write",
        path: path.to_owned(),
    })
}

/// Stops the serializer before it writes more encoded bytes than a reader accepts.
struct BoundedWriter {
    inner: BufWriter<File>,
    remaining: u64,
    exceeded: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            self.exceeded = true;
            return Err(io::Error::other("encoded record exceeds its byte limit"));
        }

        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
