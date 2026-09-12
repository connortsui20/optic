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

use rustc_hir::attrs::Linkage;
use rustc_middle::mono::MonoItemData;
use rustc_middle::mono::Visibility;
use rustc_middle::ty::SymbolName;
use rustc_session::config::Lto;
use rustc_session::config::OptLevel;
use rustc_span::Symbol;

use crate::llvm::Configuration;
use crate::protocol::END_RECORD;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::SELECTED_TARGET_MARKER_ENV;
use crate::source::Source;

/// The definition and symbol identity that rustc assigns to one monomorphized function.
pub(crate) struct ConcreteInstance<'tcx> {
    /// The crate that owns the generic or nongeneric function definition.
    pub(crate) definition_crate: Symbol,
    /// Rustc's canonical path to the function definition without generic arguments.
    pub(crate) definition_path: String,
    /// Rustc's canonical function path with this instance's concrete generic arguments.
    pub(crate) display_name: String,
    /// The symbol that identifies this concrete instance in compiler output.
    pub(crate) raw_symbol: SymbolName<'tcx>,
}

/// Writes an attempt's private manifest and publishes it only when [`Self::finish`] succeeds.
///
/// The caller **must** follow the record order and identity requirements in [`crate::protocol`].
/// The reader rejects invalid records after publication. Dropping an unfinished writer leaves
/// its output at the temporary path.
pub(crate) struct ManifestWriter {
    /// The final manifest path, made visible only after the end record is flushed.
    path: PathBuf,
    /// A sibling `.tmp` file used for all writes before publication.
    temporary_path: PathBuf,
    file: BufWriter<File>,
}

impl ManifestWriter {
    /// Creates an incomplete manifest and writes its format header.
    ///
    /// The path **must** name a file in the private attempt directory. The collector supplies the
    /// selected-target marker through [`SELECTED_TARGET_MARKER_ENV`].
    pub(crate) fn create(path: &Path) -> io::Result<Self> {
        let temporary_path = path.with_extension("tmp");
        let file = BufWriter::new(File::create(&temporary_path)?);
        let mut writer = Self {
            path: path.to_owned(),
            temporary_path,
            file,
        };

        let marker = std::env::var(SELECTED_TARGET_MARKER_ENV)
            .map_err(|error| invalid_data(error.to_string()))?;
        writer.write_bytes(&crate::protocol::header(&marker))?;

        Ok(writer)
    }

    /// Writes one function placement in the field order defined by the protocol.
    pub(crate) fn write_placement(
        &mut self,
        instance: &ConcreteInstance<'_>,
        codegen_unit: Symbol,
        placement: MonoItemData,
        source: &Source,
    ) -> io::Result<()> {
        self.write_u32(PLACEMENT_RECORD)?;

        self.write_string(instance.definition_crate.as_str())?;
        self.write_string(&instance.definition_path)?;
        self.write_string(&instance.display_name)?;
        self.write_string(instance.raw_symbol.name)?;

        self.write_string(codegen_unit.as_str())?;
        self.write_string(linkage_name(placement.linkage))?;
        self.write_string(visibility_name(placement.visibility))?;
        self.write_u32(u32::from(placement.inlined))?;
        let size_estimate = u64::try_from(placement.size_estimate).map_err(|_| {
            invalid_data(format!(
                "placement size estimate must fit in u64, got {}",
                placement.size_estimate
            ))
        })?;
        self.write_u64(size_estimate)?;

        match source {
            Source::Available(span) => {
                self.write_u32(crate::protocol::SOURCE_AVAILABLE)?;
                self.write_u64(span.artifact)?;
                self.write_u64(span.start)?;
                self.write_u64(span.length)?;
                let display_path = span
                    .display_path
                    .to_str()
                    .ok_or_else(|| invalid_data("source display path must be UTF-8"))?;
                self.write_string(display_path)?;
                self.write_u64(span.starting_line)?;
            }
            Source::Unavailable(reason) => self.write_u32(*reason)?,
        }

        Ok(())
    }

    /// Writes the single effective configuration before any source, module, or placement record.
    pub(crate) fn write_configuration(&mut self, configuration: &Configuration) -> io::Result<()> {
        let optimization = match configuration.optimization {
            OptLevel::No => "0",
            OptLevel::Less => "1",
            OptLevel::More => "2",
            OptLevel::Aggressive => "3",
            OptLevel::Size => "s",
            OptLevel::SizeMin => "z",
        };
        let lto = match &configuration.lto {
            Lto::No => crate::protocol::LTO_OFF,
            Lto::ThinLocal => crate::protocol::LTO_LOCAL_THIN,
            Lto::Thin => crate::protocol::LTO_CROSS_CRATE_THIN,
            Lto::Fat => crate::protocol::LTO_FAT,
        };

        self.write_u32(crate::protocol::CONFIGURATION_RECORD)?;
        self.write_string(&configuration.backend)?;
        self.write_string(&configuration.target)?;
        self.write_string(optimization)?;
        self.write_u32(lto)?;
        self.write_u32(u32::from(configuration.incremental))?;
        self.write_u32(u32::from(configuration.linker_plugin))?;
        self.write_u32(configuration.codegen_units)?;
        self.write_u32(crate::protocol::LLVM_RECIPE_REVISION)?;

        self.write_u32(configuration.unsupported)
    }

    /// Declares an already-written snapshot with its attempt-local ID and normalized byte length.
    ///
    /// The ID **must** be unique within this manifest. The snapshot can be empty.
    pub(crate) fn write_source_file(&mut self, id: u64, length: u64) -> io::Result<()> {
        self.write_u32(crate::protocol::SOURCE_FILE_RECORD)?;
        self.write_u64(id)?;

        self.write_u64(length)
    }

    /// Records the bitcode file that a regular codegen unit will produce after compilation.
    ///
    /// The name **must** be unique, and the path **must** come from rustc's output naming API.
    /// Unsupported configurations cannot include module records.
    pub(crate) fn write_module(&mut self, name: &str, path: &Path) -> io::Result<()> {
        self.write_u32(crate::protocol::MODULE_RECORD)?;
        self.write_string(name)?;

        let path = path
            .to_str()
            .ok_or_else(|| invalid_data("bitcode path must be UTF-8"))?;

        self.write_string(path)
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
            invalid_data(format!(
                "string length must fit in u32, got {}",
                value.len()
            ))
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

fn linkage_name(linkage: Linkage) -> &'static str {
    match linkage {
        Linkage::AvailableExternally => "AvailableExternally",
        Linkage::Common => "Common",
        Linkage::ExternalWeak => "ExternalWeak",
        Linkage::External => "External",
        Linkage::Internal => "Internal",
        Linkage::LinkOnceAny => "LinkOnceAny",
        Linkage::LinkOnceODR => "LinkOnceODR",
        Linkage::WeakAny => "WeakAny",
        Linkage::WeakODR => "WeakODR",
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Default => "Default",
        Visibility::Hidden => "Hidden",
        Visibility::Protected => "Protected",
    }
}

/// Reports a value that the private manifest format cannot encode.
fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
