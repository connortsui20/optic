//! Reconstructs durable instance records from the exact-version rustc driver.
//!
//! [`ManifestDecoder`] owns byte-level protocol validation. This module owns the semantic step that
//! groups repeated placement records by instance identity and passes each group through the
//! durable record constructors. The split keeps field-order parsing separate from record assembly.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;

use optic_records::ArtifactRecord;
use optic_records::DefinitionRecord;
use optic_records::InstanceRecord;
use optic_records::LlvmLto;
use optic_records::PlacementRecord;
use optic_records::SourceAvailability;
use optic_records::UnsupportedLlvmConfiguration;

use crate::Error;

mod decoder;
use decoder::ManifestDecoder;

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct InstanceKey {
    definition_crate: String,
    definition_path: String,
    display_name: String,
    raw_symbol: String,
}

#[derive(Debug)]
pub(crate) struct CompilerManifest {
    /// Grouped instances with their required source relationship.
    pub(crate) instances: Vec<InstanceRecord>,
    /// Source snapshots already written to generated filenames in the attempt directory.
    pub(crate) artifacts: Vec<ArtifactRecord>,
    /// Effective compiler configuration before conversion to durable provenance.
    pub(crate) configuration: Configuration,
    /// Every expected regular compiler module, independent of the function placements.
    pub(crate) modules: Vec<ExpectedModule>,
}

/// Private transport metadata. Materialization adds the exact compiler's LLVM version.
#[derive(Debug)]
pub(crate) struct Configuration {
    /// The initialized backend's reported name.
    pub(crate) backend: String,
    /// The effective target tuple or target-specification identity.
    pub(crate) target: String,
    /// The compiler's effective optimization level, not the Cargo profile name.
    pub(crate) optimization: String,
    /// The LTO mode computed by the selected compiler session.
    pub(crate) lto: LlvmLto,
    /// Whether this invocation has an incremental compilation directory.
    pub(crate) incremental: bool,
    /// Whether this invocation delegates LTO to the linker plugin.
    pub(crate) linker_plugin: bool,
    /// The configured CGU count, not the count of expected modules.
    pub(crate) codegen_units: u32,
    /// A reason classified before extra temporary files were requested.
    pub(crate) unsupported: Option<UnsupportedLlvmConfiguration>,
}

#[derive(Debug)]
pub(crate) struct ExpectedModule {
    /// The regular CGU identity provided by rustc's mono-item partition query.
    pub(crate) name: String,
    /// One rustc-derived expected file in the attempt's private LLVM directory.
    pub(crate) path: PathBuf,
}

struct InstancePlacements {
    placements: Vec<PlacementRecord>,
    source: SourceAvailability,
}

pub(crate) fn read_manifest(path: &Path, marker: &str) -> Result<CompilerManifest, Error> {
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open compiler manifest",
        path: path.to_owned(),
        source,
    })?;

    ManifestDecoder::new(path, BufReader::new(file), marker).read()
}

fn instances(placements: PlacementsByInstance) -> Result<Vec<InstanceRecord>, Error> {
    placements
        .into_iter()
        .map(|(key, placements)| {
            let definition = DefinitionRecord::new(key.definition_crate, key.definition_path)?;

            InstanceRecord::new(
                definition,
                key.display_name,
                key.raw_symbol,
                placements.placements,
                placements.source,
            )
            .map_err(Error::from)
        })
        .collect()
}

type PlacementsByInstance = BTreeMap<InstanceKey, InstancePlacements>;

#[cfg(test)]
mod tests;
