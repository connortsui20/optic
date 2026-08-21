//! Reads compiler-owned identities for concrete Rust functions.
//!
//! The isolated rustc driver writes one manifest for the selected compiler invocation. The
//! manifest stores source definition paths and raw symbols without exposing rustc types to this
//! crate.

#[cfg(test)]
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub(crate) const MANIFEST_NAME: &str = "identity-v3.bin";

const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_ID\0\0";
const PROTOCOL_VERSION: u32 = 3;
const END_RECORD: u32 = 0;
const PLACEMENT_RECORD: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024 * 1024;
#[cfg(test)]
const MAX_INSTANCES: usize = 1_000_000;
const MAX_PLACEMENTS: usize = 4_000_000;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_ARGUMENTS: usize = 65_536;

/// The source definition from which rustc instantiated a function.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DefinitionOrigin {
    /// The compiler crate that owns the definition.
    pub crate_name: String,

    /// The canonical rustc definition path without concrete generic arguments.
    pub definition_path: String,

    /// The exact source range reported by rustc, when the definition has source text.
    pub source: Option<SourceSpan>,
}

/// A half-open source byte range and its human-readable positions.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceSpan {
    /// The compiler source filename after rustc path remapping.
    pub file_name: String,

    /// The inclusive byte offset in the source file.
    pub byte_start: u64,

    /// The exclusive byte offset in the source file.
    pub byte_end: u64,

    /// The one-based starting line.
    pub line_start: usize,

    /// The zero-based starting character column.
    pub column_start: usize,

    /// The one-based ending line.
    pub line_end: usize,

    /// The zero-based ending character column.
    pub column_end: usize,
}

/// One placement of an instance in a rustc codegen unit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodegenUnitPlacement {
    /// The exact codegen-unit name reported by rustc.
    pub codegen_unit: String,

    /// The exact rustc linkage variant for this placement.
    pub linkage: String,

    /// The exact rustc visibility variant for this placement.
    pub visibility: String,

    /// Whether rustc placed a local copy rather than a globally shared instance.
    pub local_copy: bool,

    /// rustc's estimated size for this instance before code generation.
    pub size_estimate: usize,
}

/// One concrete function selected for monomorphization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompilerInstance {
    /// The definition that rustc instantiated.
    pub origin: DefinitionOrigin,

    /// The concrete Rust display name, including generic arguments.
    pub display_name: String,

    /// The exact symbol that rustc gives to LLVM.
    pub raw_symbol: String,

    /// The codegen units in which rustc placed this exact instance.
    pub placements: Vec<CodegenUnitPlacement>,
}

/// One streamed placement record from the rustc identity driver.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompilerPlacement {
    /// The definition that rustc instantiated.
    pub origin: DefinitionOrigin,

    /// The concrete Rust display name, including generic arguments.
    pub display_name: String,

    /// The exact symbol that rustc gives to LLVM.
    pub raw_symbol: String,

    /// One codegen-unit placement for this instance.
    pub placement: CodegenUnitPlacement,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct CompilerManifest {
    pub(crate) rustc_arguments: Vec<String>,
    pub(crate) instances: Vec<CompilerInstance>,
}

/// A bounded reader for compiler identity placements.
pub struct CompilerManifestReader {
    path: PathBuf,

    reader: BufReader<File>,

    rustc_arguments: Vec<String>,

    placement_count: usize,

    finished: bool,
}

impl CompilerManifestReader {
    /// Opens and validates the manifest header without reading its placement records.
    pub fn open(path: &Path, expected_rustc_commit: &str) -> Result<Self> {
        Self::open_bounded(path, expected_rustc_commit, MAX_MANIFEST_BYTES)
    }

    fn open_bounded(
        path: &Path,
        expected_rustc_commit: &str,
        maximum_manifest_bytes: u64,
    ) -> Result<Self> {
        let file = File::open(path).map_err(|source| Error::Filesystem {
            operation: "open",
            path: path.to_owned(),
            source,
        })?;
        let length = file
            .metadata()
            .map_err(|source| Error::Filesystem {
                operation: "read metadata for",
                path: path.to_owned(),
                source,
            })?
            .len();
        if length > maximum_manifest_bytes {
            return Err(invalid_manifest(
                path,
                format!("file length exceeds {maximum_manifest_bytes} bytes, got {length}"),
            ));
        }

        let mut reader = BufReader::new(file);
        validate_header(&mut reader, path, expected_rustc_commit)?;
        let argument_count = read_length_u32(&mut reader, path, "argument count", MAX_ARGUMENTS)?;
        let mut rustc_arguments = Vec::with_capacity(argument_count);

        for _ in 0..argument_count {
            rustc_arguments.push(read_string(&mut reader, path, "rustc argument")?);
        }

        Ok(Self {
            path: path.to_owned(),
            reader,
            rustc_arguments,
            placement_count: 0,
            finished: false,
        })
    }

    /// Returns the exact selected rustc command arguments from the manifest header.
    pub fn rustc_arguments(&self) -> &[String] {
        &self.rustc_arguments
    }

    /// Reads the next placement record.
    ///
    /// # Errors
    ///
    /// Returns an error if the next record is invalid or the aggregate placement bound is exceeded.
    pub fn next_placement(&mut self) -> Result<Option<CompilerPlacement>> {
        if self.finished {
            return Ok(None);
        }

        match read_u32(&mut self.reader, &self.path, "record kind")? {
            END_RECORD => {
                validate_end(&mut self.reader, &self.path)?;
                self.finished = true;

                Ok(None)
            }
            PLACEMENT_RECORD => {
                self.placement_count = self.placement_count.saturating_add(1);
                if self.placement_count > MAX_PLACEMENTS {
                    return Err(invalid_manifest(
                        &self.path,
                        format!(
                            "placement count exceeds {MAX_PLACEMENTS}, got {}",
                            self.placement_count
                        ),
                    ));
                }

                read_placement(&mut self.reader, &self.path).map(Some)
            }
            actual => Err(invalid_manifest(
                &self.path,
                format!("record kind must be 0 or 1, got {actual}"),
            )),
        }
    }
}

#[cfg(test)]
pub(crate) fn read(path: &Path, expected_rustc_commit: &str) -> Result<CompilerManifest> {
    read_bounded(path, expected_rustc_commit, MAX_MANIFEST_BYTES)
}

#[cfg(test)]
fn read_bounded(
    path: &Path,
    expected_rustc_commit: &str,
    maximum_manifest_bytes: u64,
) -> Result<CompilerManifest> {
    let mut reader =
        CompilerManifestReader::open_bounded(path, expected_rustc_commit, maximum_manifest_bytes)?;
    let rustc_arguments = reader.rustc_arguments.clone();
    let mut instance_index = HashMap::new();
    let mut instances: Vec<CompilerInstance> = Vec::new();

    while let Some(record) = reader.next_placement()? {
        let key = (
            record.origin.clone(),
            record.display_name.clone(),
            record.raw_symbol.clone(),
        );
        let index = if let Some(index) = instance_index.get(&key) {
            *index
        } else {
            if instances.len() >= MAX_INSTANCES {
                return Err(invalid_manifest(
                    path,
                    format!(
                        "instance count exceeds {MAX_INSTANCES}, got {}",
                        instances.len() + 1
                    ),
                ));
            }

            let index = instances.len();
            instances.push(CompilerInstance {
                origin: record.origin,
                display_name: record.display_name,
                raw_symbol: record.raw_symbol,
                placements: Vec::new(),
            });
            instance_index.insert(key, index);
            index
        };
        instances[index].placements.push(record.placement);
    }

    Ok(CompilerManifest {
        rustc_arguments,
        instances,
    })
}

fn validate_header(reader: &mut impl Read, path: &Path, expected_rustc_commit: &str) -> Result<()> {
    let mut magic = [0_u8; MANIFEST_MAGIC.len()];
    read_exact(reader, &mut magic, path, "manifest header")?;
    if &magic != MANIFEST_MAGIC {
        return Err(invalid_manifest(path, "invalid manifest header"));
    }

    let version = read_u32(reader, path, "protocol version")?;
    if version != PROTOCOL_VERSION {
        return Err(invalid_manifest(
            path,
            format!("unsupported protocol version, expected {PROTOCOL_VERSION}, got {version}"),
        ));
    }

    let rustc_commit = read_string(reader, path, "rustc commit")?;
    if rustc_commit != expected_rustc_commit {
        return Err(invalid_manifest(
            path,
            format!(
                "rustc commit does not match, expected {expected_rustc_commit}, got {rustc_commit}"
            ),
        ));
    }

    Ok(())
}

fn validate_end(reader: &mut impl Read, path: &Path) -> Result<()> {
    let mut trailing = [0_u8; 1];
    let trailing_length = reader
        .read(&mut trailing)
        .map_err(|source| Error::Filesystem {
            operation: "read",
            path: path.to_owned(),
            source,
        })?;
    if trailing_length != 0 {
        return Err(invalid_manifest(path, "manifest contains trailing bytes"));
    }

    Ok(())
}

fn read_placement(reader: &mut impl Read, path: &Path) -> Result<CompilerPlacement> {
    let origin = DefinitionOrigin {
        crate_name: read_string(reader, path, "definition crate")?,
        definition_path: read_string(reader, path, "definition path")?,
        source: read_optional_source_span(reader, path)?,
    };
    let display_name = read_string(reader, path, "display name")?;
    let raw_symbol = read_string(reader, path, "raw symbol")?;
    let placement = CodegenUnitPlacement {
        codegen_unit: read_string(reader, path, "codegen unit")?,
        linkage: read_string(reader, path, "codegen unit linkage")?,
        visibility: read_string(reader, path, "codegen unit visibility")?,
        local_copy: read_bool_u32(reader, path, "codegen unit local copy")?,
        size_estimate: read_usize_u64(reader, path, "codegen unit size estimate")?,
    };

    Ok(CompilerPlacement {
        origin,
        display_name,
        raw_symbol,
        placement,
    })
}

fn read_bool_u32(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<bool> {
    match read_u32(reader, path, field)? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(invalid_manifest(
            path,
            format!("{field} must be 0 or 1, got {value}"),
        )),
    }
}

fn read_optional_source_span(reader: &mut impl Read, path: &Path) -> Result<Option<SourceSpan>> {
    let present = read_u32(reader, path, "source span presence")?;
    if present == 0 {
        return Ok(None);
    }
    if present != 1 {
        return Err(invalid_manifest(
            path,
            format!("source span presence must be 0 or 1, got {present}"),
        ));
    }

    Ok(Some(SourceSpan {
        file_name: read_string(reader, path, "source filename")?,
        byte_start: read_u64(reader, path, "source byte start")?,
        byte_end: read_u64(reader, path, "source byte end")?,
        line_start: read_usize_u64(reader, path, "source line start")?,
        column_start: read_usize_u64(reader, path, "source column start")?,
        line_end: read_usize_u64(reader, path, "source line end")?,
        column_end: read_usize_u64(reader, path, "source column end")?,
    }))
}

fn read_usize_u64(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<usize> {
    let value = read_u64(reader, path, field)?;

    usize::try_from(value)
        .map_err(|_| invalid_manifest(path, format!("{field} exceeds usize, got {value}")))
}

fn read_string(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<String> {
    let length = read_length_u32(reader, path, field, MAX_STRING_BYTES)?;
    let mut bytes = vec![0_u8; length];
    read_exact(reader, &mut bytes, path, field)?;

    String::from_utf8(bytes)
        .map_err(|_| invalid_manifest(path, format!("{field} is not valid UTF-8")))
}

fn read_length_u32(
    reader: &mut impl Read,
    path: &Path,
    field: &'static str,
    maximum: usize,
) -> Result<usize> {
    let value = read_u32(reader, path, field)?;
    let value = usize::try_from(value)
        .map_err(|_| invalid_manifest(path, format!("{field} exceeds usize, got {value}")))?;
    if value > maximum {
        return Err(invalid_manifest(
            path,
            format!("{field} exceeds {maximum}, got {value}"),
        ));
    }

    Ok(value)
}

fn read_u32(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<u32> {
    let mut bytes = [0_u8; size_of::<u32>()];
    read_exact(reader, &mut bytes, path, field)?;

    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<u64> {
    let mut bytes = [0_u8; size_of::<u64>()];
    read_exact(reader, &mut bytes, path, field)?;

    Ok(u64::from_le_bytes(bytes))
}

fn read_exact(
    reader: &mut impl Read,
    bytes: &mut [u8],
    path: &Path,
    field: &'static str,
) -> Result<()> {
    reader
        .read_exact(bytes)
        .map_err(|_| invalid_manifest(path, format!("{field} is truncated")))
}

fn invalid_manifest(path: &Path, message: impl Into<String>) -> Error {
    Error::InvalidIdentityManifest {
        path: path.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        END_RECORD, MANIFEST_MAGIC, PLACEMENT_RECORD, PROTOCOL_VERSION, read, read_bounded,
    };

    #[test]
    fn reads_complete_compiler_identities() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "commit");
        manifest.extend_from_slice(&1_u32.to_le_bytes());
        push_string(&mut manifest, "rustc");
        manifest.extend_from_slice(&PLACEMENT_RECORD.to_le_bytes());
        push_string(&mut manifest, "example");
        push_string(&mut manifest, "example::kernel");
        manifest.extend_from_slice(&1_u32.to_le_bytes());
        push_string(&mut manifest, "src/lib.rs");
        manifest.extend_from_slice(&12_u64.to_le_bytes());
        manifest.extend_from_slice(&24_u64.to_le_bytes());
        manifest.extend_from_slice(&2_u64.to_le_bytes());
        manifest.extend_from_slice(&4_u64.to_le_bytes());
        manifest.extend_from_slice(&2_u64.to_le_bytes());
        manifest.extend_from_slice(&16_u64.to_le_bytes());
        push_string(&mut manifest, "example::kernel::<u64>");
        push_string(&mut manifest, "_Rexample");
        push_string(&mut manifest, "example-cgu.0");
        push_string(&mut manifest, "External");
        push_string(&mut manifest, "Default");
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&32_u64.to_le_bytes());
        manifest.extend_from_slice(&END_RECORD.to_le_bytes());
        fs::write(&path, manifest).expect("the test can write the manifest");

        let manifest = read(&path, "commit").expect("the manifest is valid");
        let instances = manifest.instances;

        assert_eq!(manifest.rustc_arguments, ["rustc"]);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].origin.crate_name, "example");
        assert_eq!(instances[0].origin.definition_path, "example::kernel");
        assert_eq!(
            instances[0]
                .origin
                .source
                .as_ref()
                .expect("the instance has source")
                .byte_start,
            12
        );
        assert_eq!(instances[0].display_name, "example::kernel::<u64>");
        assert_eq!(instances[0].raw_symbol, "_Rexample");
        assert_eq!(instances[0].placements[0].codegen_unit, "example-cgu.0");
        assert_eq!(instances[0].placements[0].linkage, "External");
        assert!(!instances[0].placements[0].local_copy);
        assert_eq!(instances[0].placements[0].size_estimate, 32);
    }

    #[test]
    fn rejects_a_different_rustc_commit() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "other");
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&END_RECORD.to_le_bytes());
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "expected").expect_err("the commit must match");

        assert!(error.to_string().contains("expected expected, got other"));
    }

    #[test]
    fn rejects_an_unsupported_protocol_version() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "commit").expect_err("the protocol version must match");

        assert!(error.to_string().contains("unsupported protocol version"));
    }

    #[test]
    fn accepts_a_manifest_at_the_aggregate_bound() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let manifest = empty_manifest("commit");
        let manifest_length =
            u64::try_from(manifest.len()).expect("the manifest length fits in u64");
        fs::write(&path, manifest).expect("the test can write the manifest");

        let manifest = read_bounded(&path, "commit", manifest_length)
            .expect("a manifest can equal the aggregate bound");

        assert!(manifest.rustc_arguments.is_empty());
        assert!(manifest.instances.is_empty());
    }

    #[test]
    fn rejects_a_manifest_over_the_aggregate_bound() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let manifest = empty_manifest("commit");
        let maximum_length =
            u64::try_from(manifest.len() - 1).expect("the manifest length fits in u64");
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read_bounded(&path, "commit", maximum_length)
            .expect_err("the aggregate bound must be enforced");

        assert!(error.to_string().contains(&format!(
            "file length exceeds {maximum_length} bytes, got {}",
            maximum_length + 1
        )));
    }

    #[test]
    fn rejects_a_truncated_string() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        manifest.extend_from_slice(&6_u32.to_le_bytes());
        manifest.extend_from_slice(b"short");
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "commit").expect_err("the string must be complete");

        assert!(error.to_string().contains("rustc commit is truncated"));
    }

    #[test]
    fn rejects_an_unknown_record_kind() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "commit");
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&2_u32.to_le_bytes());
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "commit").expect_err("the record kind must be known");

        assert!(
            error
                .to_string()
                .contains("record kind must be 0 or 1, got 2")
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "commit");
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&END_RECORD.to_le_bytes());
        manifest.push(0);
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "commit").expect_err("trailing bytes are invalid");

        assert!(error.to_string().contains("trailing bytes"));
    }

    fn push_string(manifest: &mut Vec<u8>, value: &str) {
        let length = u32::try_from(value.len()).expect("the fixture string length fits u32");
        manifest.extend_from_slice(&length.to_le_bytes());
        manifest.extend_from_slice(value.as_bytes());
    }

    fn empty_manifest(rustc_commit: &str) -> Vec<u8> {
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, rustc_commit);
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&END_RECORD.to_le_bytes());

        manifest
    }
}
