//! Streams rustc's concrete-instance placements into the private manifest format.
//!
//! Each placement repeats its instance identity so the callback can write records as rustc yields
//! them. The compiler crate groups those records after the process exits. The temporary file is
//! renamed only after the end record is flushed, so the reader never accepts partial output as a
//! completed manifest.

use std::fs;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

pub(crate) struct ConcreteInstance {
    /// The crate that owns the generic or nongeneric function definition.
    pub(crate) definition_crate: String,
    /// Rustc's canonical path to the function definition without generic arguments.
    pub(crate) definition_path: String,
    /// Rustc's canonical function path with this instance's concrete generic arguments.
    pub(crate) display_name: String,
    /// The symbol that identifies this concrete instance in compiler output.
    pub(crate) raw_symbol: String,
}

pub(crate) struct Placement {
    /// The codegen unit that contains this copy of the instance.
    pub(crate) codegen_unit: String,
    /// Rustc's linkage classification for this copy.
    pub(crate) linkage: &'static str,
    /// Rustc's symbol visibility for this copy.
    pub(crate) visibility: &'static str,
    /// Whether rustc placed this copy in the codegen unit for local use.
    pub(crate) local_copy: bool,
    /// Rustc's pre-codegen estimate of this copy's size.
    pub(crate) size_estimate: usize,
}

pub(crate) struct ManifestWriter {
    path: PathBuf,
    temporary_path: PathBuf,
    file: BufWriter<File>,
}

impl ManifestWriter {
    /// Creates an incomplete manifest and writes its format header.
    pub(crate) fn create(path: &Path) -> io::Result<Self> {
        let temporary_path = path.with_extension("tmp");
        let file = BufWriter::new(File::create(&temporary_path)?);
        let mut writer = Self {
            path: path.to_owned(),
            temporary_path,
            file,
        };
        writer.write_bytes(MANIFEST_MAGIC)?;
        writer.write_u32(PROTOCOL_VERSION)?;

        Ok(writer)
    }

    /// Writes one function placement in the field order defined by the protocol.
    pub(crate) fn write_placement(
        &mut self,
        instance: &ConcreteInstance,
        placement: &Placement,
    ) -> io::Result<()> {
        self.write_u32(PLACEMENT_RECORD)?;

        self.write_string(&instance.definition_crate)?;
        self.write_string(&instance.definition_path)?;
        self.write_string(&instance.display_name)?;
        self.write_string(&instance.raw_symbol)?;

        self.write_string(&placement.codegen_unit)?;
        self.write_string(placement.linkage)?;
        self.write_string(placement.visibility)?;
        self.write_u32(u32::from(placement.local_copy))?;
        self.write_u64(u64::try_from(placement.size_estimate).map_err(|_| {
            invalid_data(format!(
                "placement size estimate must fit in u64, got {}",
                placement.size_estimate
            ))
        })?)
    }

    /// Completes the stream and makes the final manifest path visible to the parent process.
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.write_u32(END_RECORD)?;
        self.file.flush()?;
        drop(self.file);
        fs::rename(self.temporary_path, self.path)
    }

    fn write_string(&mut self, value: &str) -> io::Result<()> {
        let length = u32::try_from(value.len()).map_err(|_| {
            invalid_data(format!("string length must fit in u32, got {}", value.len()))
        })?;
        self.write_u32(length)?;
        self.write_bytes(value.as_bytes())
    }

    fn write_u32(&mut self, value: u32) -> io::Result<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_u64(&mut self, value: u64) -> io::Result<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
