//! Reads the bounded output of the exact-version rustc driver.
//!
//! The driver protocol stays private to the compiler crate. This reader validates its complete
//! header, compiler identity, record bounds, durable record invariants, end marker, and trailing
//! bytes before collection can publish any evidence.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use optic_records::CompilerIdentity;
use optic_records::DefinitionRecord;
use optic_records::InstanceRecord;
use optic_records::PlacementRecord;

use crate::Error;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::MAX_INSTANCES;
use crate::protocol::MAX_MANIFEST_BYTES;
use crate::protocol::MAX_PLACEMENTS;
use crate::protocol::MAX_STRING_BYTES;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

/// Compiler identity and instances accepted from one complete driver manifest.
#[derive(Debug)]
pub(crate) struct CompilerOutput {
    /// The compiler identity reported and validated by the standalone driver.
    pub(crate) compiler: CompilerIdentity,
    /// The concrete instances reconstructed from placement records.
    pub(crate) instances: Vec<InstanceRecord>,
}

/// The fields that join separate placement records into one concrete instance.
#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct InstanceKey {
    definition_crate: String,
    definition_path: String,
    display_name: String,
    raw_symbol: String,
}

pub(crate) fn read_manifest(
    path: &Path,
    expected: &CompilerIdentity,
) -> Result<CompilerOutput, Error> {
    read_manifest_bounded(path, expected, MAX_MANIFEST_BYTES)
}

fn read_manifest_bounded(
    path: &Path,
    expected: &CompilerIdentity,
    maximum_manifest_bytes: u64,
) -> Result<CompilerOutput, Error> {
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open compiler manifest",
        path: path.to_owned(),
        source,
    })?;
    let length = file
        .metadata()
        .map_err(|source| Error::Filesystem {
            operation: "read compiler manifest metadata for",
            path: path.to_owned(),
            source,
        })?
        .len();
    if length > maximum_manifest_bytes {
        return Err(invalid_manifest(
            path,
            format!("file length must not exceed {maximum_manifest_bytes}, got {length}"),
        ));
    }

    let mut reader = BufReader::new(file);
    validate_magic_and_version(&mut reader, path)?;
    let compiler = read_compiler(&mut reader, path)?;
    validate_compiler(path, expected, &compiler)?;

    let placements = read_placements(&mut reader, path)?;

    let instances = placements
        .into_iter()
        .map(|(key, placements)| {
            let definition = DefinitionRecord::new(key.definition_crate, key.definition_path)?;

            InstanceRecord::new(definition, key.display_name, key.raw_symbol, placements)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CompilerOutput {
        compiler,
        instances,
    })
}

/// Reads every placement record through the end marker and groups records by instance identity.
fn read_placements(
    reader: &mut impl Read,
    path: &Path,
) -> Result<BTreeMap<InstanceKey, Vec<PlacementRecord>>, Error> {
    let mut placements = BTreeMap::new();
    let mut placement_count = 0_usize;

    loop {
        match read_u32(reader, path, "record kind")? {
            END_RECORD => {
                validate_end(reader, path)?;

                return Ok(placements);
            }
            PLACEMENT_RECORD => {
                placement_count = placement_count.saturating_add(1);
                if placement_count > MAX_PLACEMENTS {
                    return Err(invalid_manifest(
                        path,
                        format!(
                            "placement count must not exceed {MAX_PLACEMENTS}, got {placement_count}"
                        ),
                    ));
                }

                let (key, placement) = read_placement(reader, path)?;
                if !placements.contains_key(&key) && placements.len() >= MAX_INSTANCES {
                    return Err(invalid_manifest(
                        path,
                        format!(
                            "instance count must not exceed {MAX_INSTANCES}, got {}",
                            placements.len() + 1
                        ),
                    ));
                }
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

fn read_compiler(reader: &mut impl Read, path: &Path) -> Result<CompilerIdentity, Error> {
    let rustc = PathBuf::from(read_string(reader, path, "rustc path")?);
    let release = read_string(reader, path, "rustc release")?;
    let commit_hash = read_string(reader, path, "rustc commit")?;
    let host = read_string(reader, path, "rustc host")?;
    let sysroot = PathBuf::from(read_string(reader, path, "rustc sysroot")?);

    CompilerIdentity::new(rustc, release, commit_hash, host, sysroot).map_err(Error::from)
}

fn validate_compiler(
    path: &Path,
    expected: &CompilerIdentity,
    actual: &CompilerIdentity,
) -> Result<(), Error> {
    if expected.rustc() != actual.rustc() {
        return Err(invalid_manifest(
            path,
            format!(
                "rustc path must match the prepared compiler: expected {}, got {}",
                expected.rustc().display(),
                actual.rustc().display()
            ),
        ));
    }

    let mismatch = if expected.release() != actual.release() {
        Some(("release", expected.release(), actual.release()))
    } else if expected.commit_hash() != actual.commit_hash() {
        Some(("commit", expected.commit_hash(), actual.commit_hash()))
    } else if expected.host() != actual.host() {
        Some(("host", expected.host(), actual.host()))
    } else if expected.sysroot() != actual.sysroot() {
        return Err(invalid_manifest(
            path,
            format!(
                "rustc sysroot must match the prepared compiler: expected {}, got {}",
                expected.sysroot().display(),
                actual.sysroot().display()
            ),
        ));
    } else {
        None
    };

    if let Some((field, expected, actual)) = mismatch {
        return Err(invalid_manifest(
            path,
            format!(
                "rustc {field} must match the prepared compiler: expected {expected}, got {actual}"
            ),
        ));
    }

    Ok(())
}

fn read_placement(
    reader: &mut impl Read,
    path: &Path,
) -> Result<(InstanceKey, PlacementRecord), Error> {
    let crate_name = read_string(reader, path, "definition crate")?;
    let definition_path = read_string(reader, path, "definition path")?;
    let display_name = read_string(reader, path, "display name")?;
    let raw_symbol = read_string(reader, path, "raw symbol")?;
    let codegen_unit = read_string(reader, path, "codegen unit")?;
    let linkage = read_string(reader, path, "linkage")?;
    let visibility = read_string(reader, path, "visibility")?;
    let local_copy = read_bool_u32(reader, path, "local copy")?;
    let size_estimate = read_u64(reader, path, "size estimate")?;
    let placement =
        PlacementRecord::new(codegen_unit, linkage, visibility, local_copy, size_estimate)?;

    let key = InstanceKey {
        definition_crate: crate_name,
        definition_path,
        display_name,
        raw_symbol,
    };

    Ok((key, placement))
}

fn read_string(reader: &mut impl Read, path: &Path, field: &'static str) -> Result<String, Error> {
    let encoded_length = read_u32(reader, path, field)?;
    let length = usize::try_from(encoded_length).map_err(|_| {
        invalid_manifest(
            path,
            format!("{field} length must fit in usize, got {encoded_length}"),
        )
    })?;
    if length > MAX_STRING_BYTES {
        return Err(invalid_manifest(
            path,
            format!("{field} length must not exceed {MAX_STRING_BYTES}, got {length}"),
        ));
    }
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
    use std::io;
    use std::io::Read;
    use std::path::Path;

    use optic_records::CompilerIdentity;

    use crate::Error;

    use super::END_RECORD;
    use super::MANIFEST_MAGIC;
    use super::PROTOCOL_VERSION;
    use super::read_exact;
    use super::read_manifest;
    use super::read_manifest_bounded;

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture read failure",
            ))
        }
    }

    fn compiler(directory: &Path, commit_hash: &str) -> CompilerIdentity {
        compiler_at(&directory.join("rustc"), directory, commit_hash)
    }

    fn compiler_at(rustc: &Path, directory: &Path, commit_hash: &str) -> CompilerIdentity {
        CompilerIdentity::new(
            rustc.to_owned(),
            "1.0.0",
            commit_hash,
            "test-host",
            directory.join("sysroot"),
        )
        .expect("the fixture compiler identity is valid")
    }

    fn empty_manifest(compiler: &CompilerIdentity) -> Vec<u8> {
        let mut manifest = Vec::new();
        manifest.extend_from_slice(MANIFEST_MAGIC);
        manifest.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        push_string(&mut manifest, &compiler.rustc().to_string_lossy());
        push_string(&mut manifest, compiler.release());
        push_string(&mut manifest, compiler.commit_hash());
        push_string(&mut manifest, compiler.host());
        push_string(&mut manifest, &compiler.sysroot().to_string_lossy());
        manifest.extend_from_slice(&END_RECORD.to_le_bytes());

        manifest
    }

    fn push_string(manifest: &mut Vec<u8>, value: &str) {
        let length = u32::try_from(value.len()).expect("the fixture string length fits u32");
        manifest.extend_from_slice(&length.to_le_bytes());
        manifest.extend_from_slice(value.as_bytes());
    }

    #[track_caller]
    fn assert_manifest_rejected(path: &Path, expected: &CompilerIdentity, manifest: &[u8]) {
        fs::write(path, manifest).expect("the test can write the manifest");

        read_manifest(path, expected).expect_err("the manifest must be rejected");
    }

    #[test]
    fn reads_a_complete_manifest() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "commit");
        let path = temporary.path().join("manifest.bin");
        let manifest = empty_manifest(&expected);
        fs::write(&path, manifest).expect("the test can write the manifest");

        let output = read_manifest(&path, &expected).expect("the manifest is valid");

        assert_eq!(output.compiler.commit_hash(), "commit");
        assert!(output.instances.is_empty());
    }

    #[test]
    fn rejects_a_wrong_manifest_header() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "expected");
        let path = temporary.path().join("manifest.bin");
        let mut manifest = empty_manifest(&expected);
        manifest[0] ^= 1;

        assert_manifest_rejected(&path, &expected, &manifest);
    }

    #[test]
    fn rejects_a_wrong_protocol_version() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "expected");
        let path = temporary.path().join("manifest.bin");
        let mut manifest = empty_manifest(&expected);
        let version_start = MANIFEST_MAGIC.len();
        let version_end = version_start + size_of::<u32>();
        manifest[version_start..version_end].copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());

        assert_manifest_rejected(&path, &expected, &manifest);
    }

    #[test]
    fn rejects_a_different_compiler_path() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "expected");
        let path = temporary.path().join("manifest.bin");
        let actual = compiler_at(
            &temporary.path().join("other-rustc"),
            temporary.path(),
            "expected",
        );
        let manifest = empty_manifest(&actual);

        assert_manifest_rejected(&path, &expected, &manifest);
    }

    #[test]
    fn rejects_a_different_compiler_commit() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "expected");
        let path = temporary.path().join("manifest.bin");
        let manifest = empty_manifest(&compiler(temporary.path(), "other"));

        assert_manifest_rejected(&path, &expected, &manifest);
    }

    #[test]
    fn rejects_a_truncated_manifest_header() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "commit");
        let path = temporary.path().join("manifest.bin");
        let manifest = vec![0; MANIFEST_MAGIC.len() - 1];

        assert_manifest_rejected(&path, &expected, &manifest);
    }

    #[test]
    fn rejects_trailing_manifest_bytes() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "commit");
        let path = temporary.path().join("manifest.bin");
        let mut manifest = empty_manifest(&expected);
        manifest.push(0);

        assert_manifest_rejected(&path, &expected, &manifest);
    }

    #[test]
    fn rejects_a_manifest_larger_than_the_configured_bound() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let expected = compiler(temporary.path(), "commit");
        let path = temporary.path().join("manifest.bin");

        let manifest = empty_manifest(&expected);
        let maximum = u64::try_from(manifest.len() - 1).expect("the manifest length fits u64");
        fs::write(&path, manifest).expect("the test can write the manifest");

        read_manifest_bounded(&path, &expected, maximum)
            .expect_err("the aggregate bound must be enforced");
    }

    #[test]
    fn preserves_non_truncation_read_failures() {
        let mut reader = FailingReader;
        let mut byte = [0_u8; 1];
        let path = Path::new("manifest.bin");

        let error = read_exact(&mut reader, &mut byte, path, "fixture field")
            .expect_err("the read failure must remain a filesystem error");

        assert!(matches!(
            error,
            Error::Filesystem {
                operation: "read compiler manifest",
                path: error_path,
                source,
            } if error_path == path && source.kind() == io::ErrorKind::PermissionDenied
        ));
    }
}
