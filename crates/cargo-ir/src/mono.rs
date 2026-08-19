//! Reads compiler-owned identities for concrete Rust functions.
//!
//! The isolated rustc driver writes one manifest for the selected compiler invocation. The
//! manifest stores source definition paths and raw symbols without exposing rustc types to this
//! crate.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub(crate) const MANIFEST_NAME: &str = "identity-v2.bin";

const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_ID\0\0";
const PROTOCOL_VERSION: u32 = 2;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INSTANCES: usize = 1_000_000;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_CODEGEN_UNITS: usize = 65_536;
const MAX_ARGUMENTS: usize = 65_536;

/// The source definition from which rustc instantiated a function.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DefinitionOrigin {
    /// The compiler crate that owns the definition.
    pub crate_name: String,

    /// The canonical rustc definition path without concrete generic arguments.
    pub definition_path: String,

    /// The exact source range reported by rustc, when the definition has source text.
    pub source: Option<SourceSpan>,
}

/// A half-open source byte range and its human-readable positions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug)]
pub(crate) struct CompilerManifest {
    pub(crate) rustc_arguments: Vec<String>,
    pub(crate) instances: Vec<CompilerInstance>,
}

pub(crate) fn read(path: &Path, expected_rustc_commit: &str) -> Result<CompilerManifest> {
    let metadata = fs::metadata(path).map_err(|source| Error::Filesystem {
        operation: "read metadata for",
        path: path.to_owned(),
        source,
    })?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(invalid_manifest(
            path,
            format!(
                "file length exceeds {MAX_MANIFEST_BYTES} bytes, got {}",
                metadata.len()
            ),
        ));
    }

    let bytes = fs::read(path).map_err(|source| Error::Filesystem {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    let mut reader = Cursor::new(bytes.as_slice());
    let mut magic = [0_u8; MANIFEST_MAGIC.len()];
    read_exact(&mut reader, &mut magic, path, "manifest header")?;
    if &magic != MANIFEST_MAGIC {
        return Err(invalid_manifest(path, "invalid manifest header"));
    }

    let version = read_u32(&mut reader, path, "protocol version")?;
    if version != PROTOCOL_VERSION {
        return Err(invalid_manifest(
            path,
            format!("unsupported protocol version, expected {PROTOCOL_VERSION}, got {version}"),
        ));
    }

    let rustc_commit = read_string(&mut reader, path, "rustc commit")?;
    if rustc_commit != expected_rustc_commit {
        return Err(invalid_manifest(
            path,
            format!(
                "rustc commit does not match, expected {expected_rustc_commit}, got {rustc_commit}"
            ),
        ));
    }

    let argument_count = read_length_u32(&mut reader, path, "argument count", MAX_ARGUMENTS)?;
    let mut rustc_arguments = Vec::with_capacity(argument_count);

    for _ in 0..argument_count {
        rustc_arguments.push(read_string(&mut reader, path, "rustc argument")?);
    }

    let instance_count = read_length_u64(&mut reader, path, "instance count", MAX_INSTANCES)?;
    let mut instances = Vec::with_capacity(instance_count);

    for _ in 0..instance_count {
        let crate_name = read_string(&mut reader, path, "definition crate")?;
        let definition_path = read_string(&mut reader, path, "definition path")?;
        let source = read_optional_source_span(&mut reader, path)?;
        let display_name = read_string(&mut reader, path, "display name")?;
        let raw_symbol = read_string(&mut reader, path, "raw symbol")?;
        let codegen_unit_count =
            read_length_u32(&mut reader, path, "codegen unit count", MAX_CODEGEN_UNITS)?;
        let mut placements = Vec::with_capacity(codegen_unit_count);

        for _ in 0..codegen_unit_count {
            placements.push(CodegenUnitPlacement {
                codegen_unit: read_string(&mut reader, path, "codegen unit")?,
                linkage: read_string(&mut reader, path, "codegen unit linkage")?,
                visibility: read_string(&mut reader, path, "codegen unit visibility")?,
                local_copy: read_bool_u32(&mut reader, path, "codegen unit local copy")?,
                size_estimate: read_usize_u64(&mut reader, path, "codegen unit size estimate")?,
            });
        }

        instances.push(CompilerInstance {
            origin: DefinitionOrigin {
                crate_name,
                definition_path,
                source,
            },
            display_name,
            raw_symbol,
            placements,
        });
    }

    if reader.position() != bytes.len() as u64 {
        return Err(invalid_manifest(path, "manifest contains trailing bytes"));
    }

    Ok(CompilerManifest {
        rustc_arguments,
        instances,
    })
}

fn read_bool_u32(reader: &mut Cursor<&[u8]>, path: &Path, field: &'static str) -> Result<bool> {
    match read_u32(reader, path, field)? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(invalid_manifest(
            path,
            format!("{field} must be 0 or 1, got {value}"),
        )),
    }
}

fn read_optional_source_span(
    reader: &mut Cursor<&[u8]>,
    path: &Path,
) -> Result<Option<SourceSpan>> {
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

fn read_usize_u64(reader: &mut Cursor<&[u8]>, path: &Path, field: &'static str) -> Result<usize> {
    let value = read_u64(reader, path, field)?;

    usize::try_from(value)
        .map_err(|_| invalid_manifest(path, format!("{field} exceeds usize, got {value}")))
}

fn read_string(reader: &mut Cursor<&[u8]>, path: &Path, field: &'static str) -> Result<String> {
    let length = read_length_u32(reader, path, field, MAX_STRING_BYTES)?;
    let mut bytes = vec![0_u8; length];
    read_exact(reader, &mut bytes, path, field)?;

    String::from_utf8(bytes)
        .map_err(|_| invalid_manifest(path, format!("{field} is not valid UTF-8")))
}

fn read_length_u32(
    reader: &mut Cursor<&[u8]>,
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

fn read_length_u64(
    reader: &mut Cursor<&[u8]>,
    path: &Path,
    field: &'static str,
    maximum: usize,
) -> Result<usize> {
    let value = read_u64(reader, path, field)?;
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

fn read_u32(reader: &mut Cursor<&[u8]>, path: &Path, field: &'static str) -> Result<u32> {
    let mut bytes = [0_u8; size_of::<u32>()];
    read_exact(reader, &mut bytes, path, field)?;

    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut Cursor<&[u8]>, path: &Path, field: &'static str) -> Result<u64> {
    let mut bytes = [0_u8; size_of::<u64>()];
    read_exact(reader, &mut bytes, path, field)?;

    Ok(u64::from_le_bytes(bytes))
}

fn read_exact(
    reader: &mut Cursor<&[u8]>,
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

    use super::{MANIFEST_MAGIC, PROTOCOL_VERSION, read};

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
        manifest.extend_from_slice(&1_u64.to_le_bytes());
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
        manifest.extend_from_slice(&1_u32.to_le_bytes());
        push_string(&mut manifest, "example-cgu.0");
        push_string(&mut manifest, "External");
        push_string(&mut manifest, "Default");
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&32_u64.to_le_bytes());
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
        manifest.extend_from_slice(&0_u64.to_le_bytes());
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
    fn rejects_too_many_instances() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "commit");
        manifest.extend_from_slice(&0_u32.to_le_bytes());
        manifest.extend_from_slice(&1_000_001_u64.to_le_bytes());
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "commit").expect_err("the instance count must be bounded");

        assert!(error.to_string().contains("instance count exceeds 1000000"));
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
        manifest.extend_from_slice(&0_u64.to_le_bytes());
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
}
