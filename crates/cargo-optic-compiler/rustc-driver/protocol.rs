//! Defines the private protocol shared by the compiler crate and standalone driver.
//!
//! The manifest is a little-endian stream with this shape:
//!
//! ```text
//! magic, version, selected-marker string, configuration, (placement | source-file | module)*, end
//! configuration = kind, backend string, target string, optimization string, lto u32,
//!                 incremental u32, linker-plugin u32, configured-cgus u32, recipe u32, unsupported u32
//! placement = kind, four instance strings, three placement strings, local-copy u32, size u64, source
//! source = availability u32, [artifact u64, start u64, length u64, display-path string, line u64]
//! source-file = kind, artifact u64, byte-length u64
//! module = kind, compiler-module string, expected-bitcode-path string
//! string = byte-length u32, UTF-8 bytes
//! ```
//!
//! Booleans use zero or one. LTO uses 0=off, 1=local thin, 2=cross-crate thin, 3=fat. Unsupported
//! uses 0=supported, 1=incremental, 2=cross-crate thin, 3=fat, 4=linker plugin, 5=other backend,
//! 6=unverified compiler. Source availability uses 0=available with the bracketed fields,
//! 1=nonlocal, 2=unloaded, 3=generated, 4=unsupported span, 5=outside package.
//!
//! Source files use `artifact-<16-lowercase-hex-id>` in the attempt directory. Module paths come
//! from rustc's output naming API and must name immediate files in the attempt's `llvm` directory.
//! The parent converts every expected module with the matching sysroot's disassembler. No module
//! record is valid for an unsupported configuration. File and module identities must be unique.
//!
//! This protocol is private because both ends ship in the same Cargo Optic binary. Its version is
//! independent of the durable capture format. A stale receipt contains only the header. Its marker
//! binds the stopped invocation to the candidate token. Manifest readers require the same marker.

/// Identifies a file as a Cargo Optic compiler manifest before any lengths are decoded.
pub(crate) const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_1\0\0\0";
/// Identifies the header that binds a manifest or stale receipt to an analysis marker.
pub(crate) const PROTOCOL_VERSION: u32 = 3;
/// Marks the successful end of the record stream.
pub(crate) const END_RECORD: u32 = 0;
/// Marks a concrete-instance placement record.
pub(crate) const PLACEMENT_RECORD: u32 = 1;
/// Marks the single effective codegen configuration at the beginning of the stream.
pub(crate) const CONFIGURATION_RECORD: u32 = 2;
/// Declares one deduplicated normalized compiler-loaded source file.
pub(crate) const SOURCE_FILE_RECORD: u32 = 3;
/// Declares one expected regular codegen unit, including units with no function placements.
pub(crate) const MODULE_RECORD: u32 = 4;

/// Versions the verified optimized-artifact collection recipe independently of the wire format.
pub(crate) const LLVM_RECIPE_REVISION: u32 = 1;

/// Names the output path passed from the collector to the driver.
pub(crate) const MANIFEST_PATH_ENV: &str = "OPTIC_COMPILER_MANIFEST";
/// Names the unique rustc argument that identifies the selected Cargo target.
pub(crate) const SELECTED_TARGET_MARKER_ENV: &str = "OPTIC_SELECTED_TARGET_MARKER";
/// Distinguishes the inner rustc-driver process from Cargo's outer wrapper process.
pub(crate) const DRIVER_INNER_ENV: &str = "OPTIC_RUSTC_DRIVER_INNER";
/// Selects stopped probing or actual collection, with values `probe` and `collect`.
pub(crate) const MODE_ENV: &str = "OPTIC_COMPILER_MODE";
/// Names the attempt-local stale receipt. Only a verified selected probe writes this file.
pub(crate) const STALE_RECEIPT_ENV: &str = "OPTIC_STALE_RECEIPT";
/// Names the canonical compiler executable expected in Cargo's wrapper argument vector.
pub(crate) const RUSTC_ENV: &str = "OPTIC_SELECTED_RUSTC";
/// Names the canonical source path expected for the selected target.
pub(crate) const SOURCE_ENV: &str = "OPTIC_SELECTED_SOURCE";
/// Names the canonical selected package root that bounds source capture.
pub(crate) const PACKAGE_ROOT_ENV: &str = "OPTIC_SELECTED_PACKAGE_ROOT";
/// Names the rustc crate name expected for the selected target.
pub(crate) const CRATE_NAME_ENV: &str = "OPTIC_SELECTED_CRATE_NAME";
/// Reserves the selected-target argument prefix for analysis tokens.
pub(crate) const MARKER_PREFIX: &str = "--cfg=cargo_optic_selected_target=";
/// Requests the recipe key compiled into a cached driver, without entering a Cargo wrapper route.
pub(crate) const DRIVER_KEY_ARGUMENT: &str = "--optic-driver-key";

/// Encodes the common header without an end record or placement payload.
pub(crate) fn header(marker: &str) -> Vec<u8> {
    let mut bytes = MANIFEST_MAGIC.to_vec();
    bytes.extend(PROTOCOL_VERSION.to_le_bytes());
    bytes.extend(
        u32::try_from(marker.len())
            .expect("the marker contains one fixed-length token")
            .to_le_bytes(),
    );
    bytes.extend(marker.as_bytes());

    bytes
}
