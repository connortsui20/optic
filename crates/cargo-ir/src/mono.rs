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

pub(crate) const MANIFEST_NAME: &str = "identity-v1.bin";

const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_ID\0\0";
const PROTOCOL_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 1_000_000;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_CODEGEN_UNITS: usize = 65_536;

/// One concrete function selected for monomorphization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MonoItem {
    /// The source-level definition path selected by rustc.
    pub definition: String,

    /// The concrete Rust display name, including generic arguments.
    pub display_name: String,

    /// The exact symbol that rustc gives to LLVM.
    pub raw_symbol: String,

    /// The codegen units in which rustc placed the item.
    pub codegen_units: Vec<String>,
}

pub(crate) fn read(path: &Path, expected_rustc_commit: &str) -> Result<Vec<MonoItem>> {
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

    let item_count = read_length_u64(&mut reader, path, "item count", MAX_ITEMS)?;
    let mut items = Vec::with_capacity(item_count);

    for _ in 0..item_count {
        let definition = read_string(&mut reader, path, "definition")?;
        let display_name = read_string(&mut reader, path, "display name")?;
        let raw_symbol = read_string(&mut reader, path, "raw symbol")?;
        let codegen_unit_count =
            read_length_u32(&mut reader, path, "codegen unit count", MAX_CODEGEN_UNITS)?;
        let mut codegen_units = Vec::with_capacity(codegen_unit_count);

        for _ in 0..codegen_unit_count {
            codegen_units.push(read_string(&mut reader, path, "codegen unit")?);
        }

        items.push(MonoItem {
            definition,
            display_name,
            raw_symbol,
            codegen_units,
        });
    }

    if reader.position() != bytes.len() as u64 {
        return Err(invalid_manifest(path, "manifest contains trailing bytes"));
    }

    Ok(items)
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
        manifest.extend_from_slice(&1_u64.to_le_bytes());
        push_string(&mut manifest, "example::kernel");
        push_string(&mut manifest, "example::kernel::<u64>");
        push_string(&mut manifest, "_Rexample");
        manifest.extend_from_slice(&1_u32.to_le_bytes());
        push_string(&mut manifest, "example-cgu.0");
        fs::write(&path, manifest).expect("the test can write the manifest");

        let items = read(&path, "commit").expect("the manifest is valid");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].definition, "example::kernel");
        assert_eq!(items[0].display_name, "example::kernel::<u64>");
        assert_eq!(items[0].raw_symbol, "_Rexample");
        assert_eq!(items[0].codegen_units, ["example-cgu.0"]);
    }

    #[test]
    fn rejects_a_different_rustc_commit() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "other");
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
    fn rejects_too_many_items() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "commit");
        manifest.extend_from_slice(&1_000_001_u64.to_le_bytes());
        fs::write(&path, manifest).expect("the test can write the manifest");

        let error = read(&path, "commit").expect_err("the item count must be bounded");

        assert!(error.to_string().contains("item count exceeds 1000000"));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("identity.bin");
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, "commit");
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
