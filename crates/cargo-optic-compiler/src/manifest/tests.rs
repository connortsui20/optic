//! Checks the private wire format with explicit complete and malformed byte streams.
//!
//! These tests isolate decoding and durable value construction. Compiler integration tests cover
//! the real driver's writes, callback ordering, and artifact paths.

use std::fs;

use optic_records::ArtifactId;
use optic_records::ByteRange;
use optic_records::LlvmLto;
use optic_records::SourceAvailability;
use optic_records::SourceRecord;
use optic_records::SourceUnavailable;
use optic_records::UnsupportedLlvmConfiguration;

use super::CompilerManifest;
use super::read_manifest;
use crate::Error;
use crate::protocol;
use crate::protocol::END_RECORD;
use crate::protocol::MANIFEST_MAGIC;
use crate::protocol::PLACEMENT_RECORD;
use crate::protocol::PROTOCOL_VERSION;

const MARKER: &str = "--cfg=cargo_optic_selected_target=\"38a90c21244546ad8a2b3770f6cb370c\"";

/// Decodes an isolated manifest with the default marker and no external artifact dependencies.
#[track_caller]
fn decode(bytes: &[u8]) -> Result<CompilerManifest, Error> {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.bin");
    fs::write(&path, bytes).unwrap();

    read_manifest(&path, MARKER)
}

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

fn configuration(lto: u32, unsupported: u32) -> Vec<u8> {
    let mut bytes = MANIFEST_MAGIC.to_vec();
    write_u32(&mut bytes, PROTOCOL_VERSION);
    write_string(&mut bytes, MARKER);
    write_u32(&mut bytes, crate::protocol::CONFIGURATION_RECORD);

    for value in [
        "llvm",                     // The backend is LLVM.
        "x86_64-unknown-linux-gnu", // The target is x86_64 Linux.
        "3",                        // The optimization level is 3.
    ] {
        write_string(&mut bytes, value);
    }

    for value in [
        lto,                                   // Effective LTO.
        0,                                     // Incremental compilation is disabled.
        0,                                     // Linker-plugin LTO is disabled.
        16,                                    // Configured CGUs.
        crate::protocol::LLVM_RECIPE_REVISION, // Collection recipe.
        unsupported,                           // LLVM support.
    ] {
        write_u32(&mut bytes, value);
    }

    bytes
}

fn manifest(source: u32) -> Vec<u8> {
    let mut bytes = configuration(protocol::LTO_LOCAL_THIN, protocol::LLVM_SUPPORTED);
    write_u32(&mut bytes, PLACEMENT_RECORD);

    for value in [
        "fixture",                // The crate owns the definition.
        "fixture::kernel",        // This path names the definition.
        "fixture::kernel::<u64>", // This name includes concrete arguments.
        "_RNvCfixture6kernelm",   // This is the compiler's raw symbol.
        "fixture.0",              // This codegen unit contains the instance.
        "External",               // The linkage is external.
        "Default",                // The visibility is the default.
    ] {
        write_string(&mut bytes, value);
    }

    write_u32(&mut bytes, 0);
    write_u64(&mut bytes, 17);
    write_u32(&mut bytes, source);

    if source == protocol::SOURCE_AVAILABLE {
        write_u64(&mut bytes, 7);
        write_u64(&mut bytes, 2);
        write_u64(&mut bytes, 13);
        write_string(&mut bytes, "/fixture/src/lib.rs");
        write_u64(&mut bytes, 1);
        write_u32(&mut bytes, protocol::SOURCE_FILE_RECORD);
        write_u64(&mut bytes, 7);
        write_u64(&mut bytes, 15);
    }

    write_u32(&mut bytes, END_RECORD);

    bytes
}

#[test]
fn reads_a_complete_manifest() {
    let instances = decode(&manifest(protocol::SOURCE_NONLOCAL))
        .unwrap()
        .instances;

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].display_name(), "fixture::kernel::<u64>");
}

#[test]
fn rejects_a_truncated_manifest() {
    let mut bytes = manifest(protocol::SOURCE_NONLOCAL);
    bytes.pop();

    let error = decode(&bytes).unwrap_err();

    assert!(error.to_string().contains("truncated"));
}

#[test]
fn rejects_a_wrong_protocol_version() {
    let mut bytes = manifest(protocol::SOURCE_NONLOCAL);
    bytes[MANIFEST_MAGIC.len()..MANIFEST_MAGIC.len() + 4]
        .copy_from_slice(&(PROTOCOL_VERSION + 1).to_le_bytes());

    let error = decode(&bytes).unwrap_err();

    assert!(
        error
            .to_string()
            .contains(&format!("protocol version must be {PROTOCOL_VERSION}"))
    );
}

#[test]
fn rejects_trailing_bytes() {
    let mut bytes = manifest(protocol::SOURCE_NONLOCAL);
    bytes.push(1);

    let error = decode(&bytes).unwrap_err();

    assert!(error.to_string().contains("trailing bytes"));
}

#[test]
fn rejects_another_analysis_marker() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.bin");
    fs::write(&path, manifest(protocol::SOURCE_NONLOCAL)).unwrap();

    let error = read_manifest(&path, "another marker").unwrap_err();

    assert!(error.to_string().contains("selected marker must match"));
}

#[test]
fn rejects_wrong_magic_and_truncated_headers() {
    let complete = manifest(protocol::SOURCE_NONLOCAL);
    let mut wrong_magic = complete.clone();
    wrong_magic[0] = 0;

    let cases = [
        wrong_magic,             // The magic does not match.
        complete[..15].to_vec(), // The magic is truncated.
        complete[..18].to_vec(), // The version is truncated.
    ];

    for bytes in cases {
        assert!(decode(&bytes).is_err());
    }
}

#[test]
fn reads_expected_modules_without_function_placements() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest");
    let bitcode = temporary.path().join("llvm/empty.rcgu.bc");
    let mut bytes = configuration(protocol::LTO_LOCAL_THIN, protocol::LLVM_SUPPORTED);
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
        (
            protocol::LLVM_UNSUPPORTED_INCREMENTAL,
            1,
            "llvm/expected.bc",
        ), // Unsupported incremental collection.
        (protocol::LLVM_SUPPORTED, 2, "llvm/expected.bc"), // Duplicate module identity and path.
        (protocol::LLVM_SUPPORTED, 1, "outside.bc"),       // Outside the private LLVM directory.
        (protocol::LLVM_SUPPORTED, 1, "llvm/../outside.bc"), // Parent traversal.
    ] {
        let mut bytes = configuration(protocol::LTO_LOCAL_THIN, unsupported);

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

#[test]
fn unavailable_source_codes_keep_their_wire_values_and_meanings() {
    for (code, wire, reason) in [
        (protocol::SOURCE_NONLOCAL, 1, SourceUnavailable::Nonlocal), // Nonlocal definition.
        (protocol::SOURCE_UNLOADED, 2, SourceUnavailable::Unloaded), // Unloaded source.
        (protocol::SOURCE_GENERATED, 3, SourceUnavailable::Generated), // Generated definition.
        (
            protocol::SOURCE_UNSUPPORTED_SPAN,
            4,
            SourceUnavailable::UnsupportedSpan,
        ), // Unsupported span.
        (
            protocol::SOURCE_OUTSIDE_PACKAGE,
            5,
            SourceUnavailable::OutsidePackage,
        ), // Outside the package.
    ] {
        assert_eq!(code, wire);

        let manifest = decode(&manifest(code)).unwrap();

        assert_eq!(
            manifest.instances[0].source(),
            &SourceAvailability::Unavailable(reason)
        );
    }
}

#[test]
fn available_source_code_keeps_its_wire_value_and_payload() {
    assert_eq!(protocol::SOURCE_AVAILABLE, 0);

    let manifest = decode(&manifest(protocol::SOURCE_AVAILABLE)).unwrap();
    let expected = SourceRecord::new(
        ArtifactId::new(7),
        ByteRange::new(2, 13).unwrap(),
        "/fixture/src/lib.rs".into(),
        1,
    )
    .unwrap();

    assert_eq!(
        manifest.instances[0].source(),
        &SourceAvailability::Available(expected)
    );
    assert_eq!(manifest.artifacts.len(), 1);
}

#[test]
fn lto_codes_keep_their_wire_values_and_meanings() {
    for (code, wire, expected) in [
        (protocol::LTO_OFF, 0, LlvmLto::Off),              // No LTO.
        (protocol::LTO_LOCAL_THIN, 1, LlvmLto::LocalThin), // Local ThinLTO.
        (protocol::LTO_CROSS_CRATE_THIN, 2, LlvmLto::CrossCrateThin), // Cross-crate ThinLTO.
        (protocol::LTO_FAT, 3, LlvmLto::Fat),              // Fat LTO.
    ] {
        assert_eq!(code, wire);

        let mut bytes = configuration(code, protocol::LLVM_SUPPORTED);
        write_u32(&mut bytes, END_RECORD);

        assert_eq!(decode(&bytes).unwrap().configuration.lto, expected);
    }
}

#[test]
fn llvm_support_codes_keep_their_wire_values_and_meanings() {
    for (code, wire, expected) in [
        (protocol::LLVM_SUPPORTED, 0, None), // Supported configuration.
        (
            protocol::LLVM_UNSUPPORTED_INCREMENTAL,
            1,
            Some(UnsupportedLlvmConfiguration::Incremental),
        ), // Incremental compilation.
        (
            protocol::LLVM_UNSUPPORTED_CROSS_CRATE_THIN,
            2,
            Some(UnsupportedLlvmConfiguration::CrossCrateThinLto),
        ), // Cross-crate ThinLTO.
        (
            protocol::LLVM_UNSUPPORTED_FAT,
            3,
            Some(UnsupportedLlvmConfiguration::FatLto),
        ), // Fat LTO.
        (
            protocol::LLVM_UNSUPPORTED_LINKER_PLUGIN,
            4,
            Some(UnsupportedLlvmConfiguration::LinkerPluginLto),
        ), // Linker-plugin LTO.
        (
            protocol::LLVM_UNSUPPORTED_OTHER_BACKEND,
            5,
            Some(UnsupportedLlvmConfiguration::OtherBackend),
        ), // Another backend.
        (
            protocol::LLVM_UNSUPPORTED_UNVERIFIED_COMPILER,
            6,
            Some(UnsupportedLlvmConfiguration::UnverifiedCompiler),
        ), // Unverified compiler.
    ] {
        assert_eq!(code, wire);

        let mut bytes = configuration(protocol::LTO_LOCAL_THIN, code);
        write_u32(&mut bytes, END_RECORD);

        assert_eq!(decode(&bytes).unwrap().configuration.unsupported, expected);
    }
}

#[test]
fn rejects_unknown_source_lto_and_llvm_support_codes() {
    let mut unknown_lto = configuration(u32::MAX, protocol::LLVM_SUPPORTED);
    write_u32(&mut unknown_lto, END_RECORD);
    let mut unknown_support = configuration(protocol::LTO_LOCAL_THIN, u32::MAX);
    write_u32(&mut unknown_support, END_RECORD);

    for (bytes, field) in [
        (manifest(u32::MAX), "source availability"), // Unknown source availability.
        (unknown_lto, "LTO"),                        // Unknown LTO mode.
        (unknown_support, "unsupported configuration"), // Unknown LLVM support.
    ] {
        let error = decode(&bytes).unwrap_err();

        assert!(
            error.to_string().contains(&format!(
                "{field} must be a known protocol code, got {}",
                u32::MAX
            )),
            "{error}"
        );
    }
}
