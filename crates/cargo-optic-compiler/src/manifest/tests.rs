use std::fs;

use super::read_manifest;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

const MARKER: &str = "--cfg=cargo_optic_selected_target=\"38a90c21244546ad8a2b3770f6cb370c\"";

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

fn configuration(unsupported: u32) -> Vec<u8> {
    let mut bytes = MANIFEST_MAGIC.to_vec();
    write_u32(&mut bytes, PROTOCOL_VERSION);
    write_string(&mut bytes, MARKER);
    write_u32(&mut bytes, crate::protocol::CONFIGURATION_RECORD);
    for value in ["llvm", "x86_64-unknown-linux-gnu", "3"] {
        write_string(&mut bytes, value);
    }
    for value in [
        1,
        0,
        0,
        16,
        crate::protocol::LLVM_RECIPE_REVISION,
        unsupported,
    ] {
        write_u32(&mut bytes, value);
    }

    bytes
}

fn manifest() -> Vec<u8> {
    let mut bytes = configuration(0);
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
    write_u32(&mut bytes, 1);
    write_u32(&mut bytes, END_RECORD);

    bytes
}

#[test]
fn reads_a_complete_manifest() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.bin");
    fs::write(&path, manifest()).unwrap();

    let instances = read_manifest(&path, MARKER).unwrap().instances;

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

    let error = read_manifest(&path, MARKER).unwrap_err();

    assert!(error.to_string().contains("truncated"));
}

#[test]
fn rejects_a_wrong_protocol_version() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.bin");
    let mut bytes = manifest();
    bytes[MANIFEST_MAGIC.len()..MANIFEST_MAGIC.len() + 4]
        .copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());
    fs::write(&path, bytes).unwrap();

    let error = read_manifest(&path, MARKER).unwrap_err();

    assert!(
        error
            .to_string()
            .contains(&format!("protocol version must be {PROTOCOL_VERSION}"))
    );
}

#[test]
fn rejects_trailing_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.bin");
    let mut bytes = manifest();
    bytes.push(1);
    fs::write(&path, bytes).unwrap();

    let error = read_manifest(&path, MARKER).unwrap_err();

    assert!(error.to_string().contains("trailing bytes"));
}

#[test]
fn rejects_another_analysis_marker() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.bin");
    fs::write(&path, manifest()).unwrap();

    let error = read_manifest(&path, "another marker").unwrap_err();

    assert!(error.to_string().contains("selected marker must match"));
}

#[test]
fn rejects_wrong_magic_and_truncated_headers() {
    let complete = manifest();
    let mut wrong_magic = complete.clone();
    wrong_magic[0] = 0;
    let cases = [
        wrong_magic,
        complete[..15].to_vec(),
        complete[..18].to_vec(),
    ];

    for bytes in cases {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("manifest.bin");
        fs::write(&path, bytes).unwrap();

        assert!(read_manifest(&path, MARKER).is_err());
    }
}

#[test]
fn reads_expected_modules_without_function_placements() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest");
    let bitcode = temporary.path().join("llvm/empty.rcgu.bc");
    let mut bytes = configuration(0);
    write_u32(&mut bytes, crate::protocol::MODULE_RECORD);
    write_string(&mut bytes, "empty");
    write_string(&mut bytes, bitcode.to_str().unwrap());
    write_u32(&mut bytes, END_RECORD);
    fs::write(&path, bytes).unwrap();

    let manifest = read_manifest(&path, MARKER).unwrap();
    assert!(manifest.instances.is_empty());
    assert_eq!(manifest.modules.len(), 1);
    assert_eq!(manifest.modules[0].name, "empty");
    assert_eq!(manifest.modules[0].path, bitcode);
}

#[test]
fn rejects_unsupported_duplicate_and_outside_module_paths() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest");

    for (unsupported, count, name) in [
        (1, 1, "llvm/expected.bc"),   // Unsupported incremental collection.
        (0, 2, "llvm/expected.bc"),   // Duplicate module identity and path.
        (0, 1, "outside.bc"),         // Outside the private LLVM directory.
        (0, 1, "llvm/../outside.bc"), // Parent traversal.
    ] {
        let mut bytes = configuration(unsupported);
        for _ in 0..count {
            write_u32(&mut bytes, crate::protocol::MODULE_RECORD);
            write_string(&mut bytes, "module");
            write_string(&mut bytes, temporary.path().join(name).to_str().unwrap());
        }
        write_u32(&mut bytes, END_RECORD);
        fs::write(&path, bytes).unwrap();
        assert!(read_manifest(&path, MARKER).is_err());
    }
}
