//! Reconstructs durable instance records from the exact-version rustc driver.
//!
//! [`ManifestDecoder`] owns byte-level protocol validation. This module owns the semantic step that
//! groups repeated placement records by instance identity and passes each group through the
//! durable record constructors. The split keeps field-order parsing separate from record assembly.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use optic_records::DefinitionRecord;
use optic_records::InstanceRecord;
use optic_records::PlacementRecord;

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

pub(crate) fn read_manifest(path: &Path) -> Result<Vec<InstanceRecord>, Error> {
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open compiler manifest",
        path: path.to_owned(),
        source,
    })?;
    let placements = ManifestDecoder::new(path, BufReader::new(file)).read()?;

    placements
        .into_iter()
        .map(|(key, placements)| {
            let definition = DefinitionRecord::new(key.definition_crate, key.definition_path)?;

            InstanceRecord::new(definition, key.display_name, key.raw_symbol, placements)
                .map_err(Error::from)
        })
        .collect()
}

type PlacementsByInstance = BTreeMap<InstanceKey, Vec<PlacementRecord>>;

#[cfg(test)]
mod tests;
