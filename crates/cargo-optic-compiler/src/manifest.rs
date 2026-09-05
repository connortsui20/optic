//! Reads the output of the exact-version rustc driver.
//!
//! The private protocol detects incomplete or incompatible driver output. Durable record
//! constructors validate each reconstructed instance before collection returns it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::path::Path;

use optic_records::DefinitionRecord;
use optic_records::InstanceRecord;
use optic_records::PlacementRecord;

use crate::Error;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

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
    let mut reader = BufReader::new(file);
    validate_magic_and_version(&mut reader, path)?;
    let placements = read_placements(&mut reader, path)?;

    placements
        .into_iter()
        .map(|(key, placements)| {
            let definition = DefinitionRecord::new(key.definition_crate, key.definition_path)?;

            InstanceRecord::new(definition, key.display_name, key.raw_symbol, placements)
                .map_err(Error::from)
        })
        .collect()
}

fn read_placements(
    reader: &mut impl Read,
    path: &Path,
) -> Result<BTreeMap<InstanceKey, Vec<PlacementRecord>>, Error> {
    let mut placements = BTreeMap::new();

    loop {
        match read_u32(reader, path, "record kind")? {
            END_RECORD => {
                validate_end(reader, path)?;

                return Ok(placements);
            }
            PLACEMENT_RECORD => {
                let (key, placement) = read_placement(reader, path)?;
                placements.entry(key).or_default().push(placement);
            }
            actual => {
                return Err(invalid_manifest(
                    path,
                    format!("record kind must be 0 or 1, got {actual}"),
                ));
            }
        }
    }
}

fn validate_magic_and_version(reader: &mut impl Read, path: &Path) -> Result<(), Error> {
    let mut magic = [0_u8; MANIFEST_MAGIC.len()];
    read_exact(reader, &mut magic, path, "manifest header")?;
    if &magic != MANIFEST_MAGIC {
        return Err(invalid_manifest(
            path,
            format!("manifest header must match Cargo Optic, got {magic:?}"),
        ));
    }

    let version = read_u32(reader, path, "protocol version")?;
    if version != PROTOCOL_VERSION {
        return Err(invalid_manifest(
            path,
            format!("protocol version must be {PROTOCOL_VERSION}, got {version}"),
        ));
    }

    Ok(())
}

fn read_placement(
    reader: &mut impl Read,
    path: &Path,
) -> Result<(InstanceKey, PlacementRecord), Error> {
    let key = InstanceKey {
        definition_crate: read_string(reader, path, "definition crate")?,
        definition_path: read_string(reader, path, "definition path")?,
        display_name: read_string(reader, path, "display name")?,
        raw_symbol: read_string(reader, path, "raw symbol")?,
    };
    let codegen_unit = read_string(reader, path, "codegen unit")?;
    let linkage = read_string(reader, path, "linkage")?;
    let visibility = read_string(reader, path, "visibility")?;
    let local_copy = read_bool_u32(reader, path, "local copy")?;
    let size_estimate = read_u64(reader, path, "size estimate")?;
    let placement =
        PlacementRecord::new(codegen_unit, linkage, visibility, local_copy, size_estimate)?;

    Ok((key, placement))
}

fn read_string(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<String, Error> {
    let length = usize::try_from(read_u32(reader, path, field)?).map_err(|error| {
        invalid_manifest(path, format!("{field} length must fit in usize: {error}"))
    })?;
    let mut bytes = vec![0_u8; length];
    read_exact(reader, &mut bytes, path, field)?;

    String::from_utf8(bytes).map_err(|error| {
        invalid_manifest(path, format!("{field} must be valid UTF-8, got {error}"))
    })
}

fn read_bool_u32(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<bool, Error> {
    match read_u32(reader, path, field)? {
        0 => Ok(false),
        1 => Ok(true),
        actual => Err(invalid_manifest(
            path,
            format!("{field} must be 0 or 1, got {actual}"),
        )),
    }
}

fn read_u32(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<u32, Error> {
    let mut bytes = [0_u8; size_of::<u32>()];
    read_exact(reader, &mut bytes, path, field)?;

    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<u64, Error> {
    let mut bytes = [0_u8; size_of::<u64>()];
    read_exact(reader, &mut bytes, path, field)?;

    Ok(u64::from_le_bytes(bytes))
}

fn read_exact(
    reader: &mut impl Read,
    bytes: &mut [u8],
    path: &Path,
    field: &'static str,
) -> Result<(), Error> {
    match reader.read_exact(bytes) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::UnexpectedEof => Err(invalid_manifest(
            path,
            format!("{field} must not be truncated, got end of file"),
        )),
        Err(source) => Err(Error::Filesystem {
            operation: "read compiler manifest",
            path: path.to_owned(),
            source,
        }),
    }
}

fn validate_end(reader: &mut impl Read, path: &Path) -> Result<(), Error> {
    let mut trailing = [0_u8; 1];
    let length = reader
        .read(&mut trailing)
        .map_err(|source| Error::Filesystem {
            operation: "read compiler manifest",
            path: path.to_owned(),
            source,
        })?;
    if length != 0 {
        return Err(invalid_manifest(
            path,
            "manifest must not contain trailing bytes, got at least one trailing byte",
        ));
    }

    Ok(())
}

fn invalid_manifest(path: &Path, message: impl Into<String>) -> Error {
    Error::InvalidManifest {
        path: path.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::END_RECORD;
    use super::MANIFEST_MAGIC;
    use super::PLACEMENT_RECORD;
    use super::PROTOCOL_VERSION;
    use super::read_manifest;

    fn write_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn write_u64(bytes: &mut Vec<u8>, value: u64) {
        bytes.extend(value.to_le_bytes());
    }

    fn write_string(bytes: &mut Vec<u8>, value: &str) {
        write_u32(bytes, u32::try_from(value.len()).unwrap());
        bytes.extend(value.as_bytes());
    }

    fn manifest() -> Vec<u8> {
        let mut bytes = MANIFEST_MAGIC.to_vec();
        write_u32(&mut bytes, PROTOCOL_VERSION);
        write_u32(&mut bytes, PLACEMENT_RECORD);
        for value in [
            "fixture",
            "fixture::kernel",
            "fixture::kernel::<u64>",
            "_RNvCfixture6kernelm",
            "fixture.0",
            "External",
            "Default",
        ] {
            write_string(&mut bytes, value);
        }
        write_u32(&mut bytes, 0);
        write_u64(&mut bytes, 17);
        write_u32(&mut bytes, END_RECORD);

        bytes
    }

    #[test]
    fn reads_a_complete_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("manifest.bin");
        fs::write(&path, manifest()).unwrap();

        let instances = read_manifest(&path).unwrap();

        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].display_name(), "fixture::kernel::<u64>");
    }

    #[test]
    fn rejects_a_truncated_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("manifest.bin");
        let mut bytes = manifest();
        bytes.pop();
        fs::write(&path, bytes).unwrap();

        let error = read_manifest(&path).unwrap_err();

        assert!(error.to_string().contains("truncated"));
    }

    #[test]
    fn rejects_a_wrong_protocol_version() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("manifest.bin");
        let mut bytes = manifest();
        bytes[MANIFEST_MAGIC.len()..MANIFEST_MAGIC.len() + 4].copy_from_slice(&2_u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();

        let error = read_manifest(&path).unwrap_err();

        assert!(error.to_string().contains("protocol version must be 1"));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("manifest.bin");
        let mut bytes = manifest();
        bytes.push(1);
        fs::write(&path, bytes).unwrap();

        let error = read_manifest(&path).unwrap_err();

        assert!(error.to_string().contains("trailing bytes"));
    }
}
