//! Decodes and validates the private compiler-manifest byte stream.
//!
//! [`ManifestDecoder`] keeps the source path with its reader for protocol and I/O error context.
//! Record constructors validate decoded values. The methods follow the wire format from header to
//! end marker, which makes the decoder readable in the same order as the protocol documentation.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use optic_records::ArtifactId;
use optic_records::ArtifactKind;
use optic_records::ArtifactRecord;
use optic_records::ByteRange;
use optic_records::LlvmLto;
use optic_records::PlacementRecord;
use optic_records::SourceAvailability;
use optic_records::SourceRecord;
use optic_records::SourceUnavailable;
use optic_records::UnsupportedLlvmConfiguration;

use super::CompilerManifest;
use super::Configuration;
use super::ExpectedModule;
use super::InstanceKey;
use super::InstancePlacements;
use super::PlacementsByInstance;
use crate::Error;
use crate::protocol;
use crate::protocol::CONFIGURATION_RECORD;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::MODULE_RECORD;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;
use crate::protocol::SOURCE_FILE_RECORD;

/// Decodes one invocation's stream while retaining its path for error context.
pub(super) struct ManifestDecoder<R> {
    path: PathBuf,
    reader: R,
    marker: String,
}

impl<R: Read> ManifestDecoder<R> {
    /// Associates a stream with the path and selected marker used by its collection attempt.
    pub(super) fn new(path: &Path, reader: R, marker: &str) -> Self {
        Self {
            path: path.to_owned(),
            reader,
            marker: marker.to_owned(),
        }
    }

    /// Reads one complete manifest and rejects partial or trailing data.
    pub(super) fn read(mut self) -> Result<CompilerManifest, Error> {
        self.read_header()?;
        let configuration = self.read_configuration()?;

        let mut placements = PlacementsByInstance::new();
        let mut artifacts = Vec::new();
        let mut artifact_ids = HashSet::new();
        let mut modules = Vec::new();
        let mut module_names = HashSet::new();
        let mut module_paths = HashSet::new();

        loop {
            match self.read_u32("record kind")? {
                END_RECORD => {
                    self.require_end_of_file()?;

                    return Ok(CompilerManifest {
                        instances: super::assemble_instances(placements)?,
                        artifacts,
                        configuration,
                        modules,
                    });
                }
                PLACEMENT_RECORD => self.read_instance_placement(&mut placements)?,
                SOURCE_FILE_RECORD => {
                    artifacts.push(self.read_source_file(&mut artifact_ids)?);
                }
                MODULE_RECORD => modules.push(self.read_expected_module(
                    &configuration,
                    &mut module_names,
                    &mut module_paths,
                )?),
                actual => {
                    return Err(self.invalid(format!(
                        "record kind must identify an end, placement, source file, or module, \
                         got {actual}"
                    )));
                }
            }
        }
    }

    fn read_instance_placement(
        &mut self,
        placements: &mut PlacementsByInstance,
    ) -> Result<(), Error> {
        let (key, placement) = self.read_placement()?;
        let source = self.read_source()?;

        if let Some(existing) = placements.get_mut(&key) {
            if existing.source != source {
                return Err(self.invalid(
                    "one instance must have one source relationship, \
                     got conflicting source records",
                ));
            }

            existing.placements.push(placement);

            return Ok(());
        }

        placements.insert(
            key,
            InstancePlacements {
                placements: vec![placement],
                source,
            },
        );

        Ok(())
    }

    fn read_source_file(
        &mut self,
        artifact_ids: &mut HashSet<ArtifactId>,
    ) -> Result<ArtifactRecord, Error> {
        let id = ArtifactId::new(self.read_u64("source artifact ID")?);
        let length = self.read_u64("source byte length")?;

        if !artifact_ids.insert(id) {
            return Err(self.invalid("source artifact IDs must be unique, got a duplicate ID"));
        }

        Ok(ArtifactRecord::new(id, ArtifactKind::Source, length))
    }

    fn read_expected_module(
        &mut self,
        configuration: &Configuration,
        module_names: &mut HashSet<String>,
        module_paths: &mut HashSet<PathBuf>,
    ) -> Result<ExpectedModule, Error> {
        let name = self.read_string("compiler module")?;
        let path = PathBuf::from(self.read_string("expected bitcode path")?);

        if configuration.unsupported.is_some() {
            return Err(self.invalid("unsupported LLVM requires no module records, got a module"));
        }

        let directory = self
            .path
            .parent()
            .expect("collection supplies a manifest path inside its attempt directory")
            .join("llvm");

        if name.is_empty()
            || path.parent() != Some(directory.as_path())
            || path.file_name().is_none()
        {
            return Err(self.invalid(format!(
                "expected bitcode must be an immediate private LLVM file, got {}",
                path.display()
            )));
        }

        if !module_names.insert(name.clone()) || !module_paths.insert(path.clone()) {
            return Err(self.invalid("expected modules and paths must be unique, got a duplicate"));
        }

        Ok(ExpectedModule { name, path })
    }

    fn read_configuration(&mut self) -> Result<Configuration, Error> {
        let kind = self.read_u32("configuration record kind")?;

        if kind != CONFIGURATION_RECORD {
            return Err(self.invalid(format!(
                "the first record must be configuration, got {kind}"
            )));
        }

        let backend = self.read_string("codegen backend")?;
        let target = self.read_string("effective target")?;
        let optimization = self.read_string("optimization level")?;

        let lto = match self.read_u32("effective LTO")? {
            protocol::LTO_OFF => LlvmLto::Off,
            protocol::LTO_LOCAL_THIN => LlvmLto::LocalThin,
            protocol::LTO_CROSS_CRATE_THIN => LlvmLto::CrossCrateThin,
            protocol::LTO_FAT => LlvmLto::Fat,
            actual => {
                return Err(
                    self.invalid(format!("LTO must be a known protocol code, got {actual}"))
                );
            }
        };

        let incremental = self.read_bool("incremental compilation")?;
        let linker_plugin = self.read_bool("linker-plugin LTO")?;
        let codegen_units = self.read_u32("configured codegen units")?;
        let recipe = self.read_u32("LLVM recipe revision")?;

        if recipe != crate::protocol::LLVM_RECIPE_REVISION {
            return Err(self.invalid(format!(
                "LLVM recipe revision must match this collector, got {recipe}"
            )));
        }

        let unsupported = match self.read_u32("unsupported LLVM configuration")? {
            protocol::LLVM_SUPPORTED => None,
            protocol::LLVM_UNSUPPORTED_INCREMENTAL => {
                Some(UnsupportedLlvmConfiguration::Incremental)
            }
            protocol::LLVM_UNSUPPORTED_CROSS_CRATE_THIN => {
                Some(UnsupportedLlvmConfiguration::CrossCrateThinLto)
            }
            protocol::LLVM_UNSUPPORTED_FAT => Some(UnsupportedLlvmConfiguration::FatLto),
            protocol::LLVM_UNSUPPORTED_LINKER_PLUGIN => {
                Some(UnsupportedLlvmConfiguration::LinkerPluginLto)
            }
            protocol::LLVM_UNSUPPORTED_OTHER_BACKEND => {
                Some(UnsupportedLlvmConfiguration::OtherBackend)
            }
            protocol::LLVM_UNSUPPORTED_UNVERIFIED_COMPILER => {
                Some(UnsupportedLlvmConfiguration::UnverifiedCompiler)
            }
            actual => {
                return Err(self.invalid(format!(
                    "unsupported configuration must be a known protocol code, got {actual}"
                )));
            }
        };

        Ok(Configuration {
            backend,
            target,
            optimization,
            lto,
            incremental,
            linker_plugin,
            codegen_units,
            unsupported,
        })
    }

    fn read_source(&mut self) -> Result<SourceAvailability, Error> {
        let reason = match self.read_u32("source availability")? {
            protocol::SOURCE_AVAILABLE => {
                let artifact = ArtifactId::new(self.read_u64("source artifact")?);
                let start = self.read_u64("source start")?;
                let length = self.read_u64("source length")?;
                let path = PathBuf::from(self.read_string("source display path")?);
                let line = self.read_u64("source starting line")?;

                return Ok(SourceAvailability::Available(SourceRecord::new(
                    artifact,
                    ByteRange::new(start, length)?,
                    path,
                    line,
                )?));
            }
            protocol::SOURCE_NONLOCAL => SourceUnavailable::Nonlocal,
            protocol::SOURCE_UNLOADED => SourceUnavailable::Unloaded,
            protocol::SOURCE_GENERATED => SourceUnavailable::Generated,
            protocol::SOURCE_UNSUPPORTED_SPAN => SourceUnavailable::UnsupportedSpan,
            protocol::SOURCE_OUTSIDE_PACKAGE => SourceUnavailable::OutsidePackage,
            actual => {
                return Err(self.invalid(format!(
                    "source availability must be a known protocol code, got {actual}"
                )));
            }
        };

        Ok(SourceAvailability::Unavailable(reason))
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

        let marker = self.read_string("selected marker")?;

        if marker != self.marker {
            return Err(self.invalid(format!(
                "selected marker must match this analysis, got {marker:?}"
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
