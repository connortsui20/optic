//! Decodes and validates the private compiler-manifest byte stream.
//!
//! [`ManifestDecoder`] keeps the source path with its reader so every malformed field and I/O
//! failure reports the manifest that caused it. The methods follow the wire format from header to
//! end marker, which makes the decoder readable in the same order as the protocol documentation.

use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use optic_records::PlacementRecord;

use super::InstanceKey;
use super::PlacementsByInstance;
use crate::Error;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

pub(super) struct ManifestDecoder<R> {
    path: PathBuf,
    reader: R,
}

impl<R: Read> ManifestDecoder<R> {
    pub(super) fn new(path: &Path, reader: R) -> Self {
        Self {
            path: path.to_owned(),
            reader,
        }
    }

    /// Reads one complete manifest and rejects partial or trailing data.
    pub(super) fn read(mut self) -> Result<PlacementsByInstance, Error> {
        self.read_header()?;

        let mut placements = PlacementsByInstance::new();
        loop {
            match self.read_u32("record kind")? {
                END_RECORD => {
                    self.require_end_of_file()?;

                    return Ok(placements);
                }
                PLACEMENT_RECORD => {
                    let (key, placement) = self.read_placement()?;
                    placements.entry(key).or_default().push(placement);
                }
                actual => {
                    return Err(self.invalid(format!(
                        "record kind must be {END_RECORD} or {PLACEMENT_RECORD}, got {actual}"
                    )));
                }
            }
        }
    }

    fn read_header(&mut self) -> Result<(), Error> {
        let mut magic = [0_u8; MANIFEST_MAGIC.len()];
        self.read_exact(&mut magic, "manifest header")?;
        if &magic != MANIFEST_MAGIC {
            return Err(self.invalid(format!(
                "manifest header must match Cargo Optic, got {magic:?}"
            )));
        }

        let version = self.read_u32("protocol version")?;
        if version != PROTOCOL_VERSION {
            return Err(self.invalid(format!(
                "protocol version must be {PROTOCOL_VERSION}, got {version}"
            )));
        }

        Ok(())
    }

    /// Reads the fields in the order documented by the shared protocol module.
    fn read_placement(&mut self) -> Result<(InstanceKey, PlacementRecord), Error> {
        let key = InstanceKey {
            definition_crate: self.read_string("definition crate")?,
            definition_path: self.read_string("definition path")?,
            display_name: self.read_string("display name")?,
            raw_symbol: self.read_string("raw symbol")?,
        };

        let codegen_unit = self.read_string("codegen unit")?;
        let linkage = self.read_string("linkage")?;
        let visibility = self.read_string("visibility")?;
        let local_copy = self.read_bool("local copy")?;
        let size_estimate = self.read_u64("size estimate")?;
        let placement =
            PlacementRecord::new(codegen_unit, linkage, visibility, local_copy, size_estimate)?;

        Ok((key, placement))
    }

    fn read_string(&mut self, field: &'static str) -> Result<String, Error> {
        let length = usize::try_from(self.read_u32(field)?)
            .map_err(|error| self.invalid(format!("{field} length must fit in usize: {error}")))?;
        let mut bytes = vec![0_u8; length];
        self.read_exact(&mut bytes, field)?;

        String::from_utf8(bytes)
            .map_err(|error| self.invalid(format!("{field} must be valid UTF-8, got {error}")))
    }

    fn read_bool(&mut self, field: &'static str) -> Result<bool, Error> {
        match self.read_u32(field)? {
            0 => Ok(false),
            1 => Ok(true),
            actual => Err(self.invalid(format!("{field} must be 0 or 1, got {actual}"))),
        }
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, Error> {
        let mut bytes = [0_u8; size_of::<u32>()];
        self.read_exact(&mut bytes, field)?;

        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&mut self, field: &'static str) -> Result<u64, Error> {
        let mut bytes = [0_u8; size_of::<u64>()];
        self.read_exact(&mut bytes, field)?;

        Ok(u64::from_le_bytes(bytes))
    }

    fn read_exact(&mut self, bytes: &mut [u8], field: &'static str) -> Result<(), Error> {
        match self.reader.read_exact(bytes) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::UnexpectedEof => {
                Err(self.invalid(format!("{field} must not be truncated, got end of file")))
            }
            Err(source) => Err(Error::Filesystem {
                operation: "read compiler manifest",
                path: self.path.clone(),
                source,
            }),
        }
    }

    fn require_end_of_file(&mut self) -> Result<(), Error> {
        let mut trailing = [0_u8; 1];
        let length = self
            .reader
            .read(&mut trailing)
            .map_err(|source| Error::Filesystem {
                operation: "read compiler manifest",
                path: self.path.clone(),
                source,
            })?;
        if length != 0 {
            return Err(self.invalid(
                "manifest must not contain trailing bytes, got at least one trailing byte",
            ));
        }

        Ok(())
    }

    fn invalid(&self, message: impl Into<String>) -> Error {
        Error::InvalidManifest {
            path: self.path.clone(),
            message: message.into(),
        }
    }
}
