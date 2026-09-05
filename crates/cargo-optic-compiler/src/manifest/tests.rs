use std::fs;

use super::read_manifest;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

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
