//! Defines the private protocol shared by the compiler crate and standalone driver.
//!
//! The manifest is a little-endian stream with this shape:
//!
//! ```text
//! magic, version, selected-marker string, configuration, (placement | source-file | module)*, end
//! configuration = kind, backend string, target string, optimization string, lto u32,
//!                 incremental u32, linker-plugin u32, configured-cgus u32, recipe u32,
//!                 unsupported u32
//! placement = kind, four instance strings, three placement strings, local-copy u32, size u64,
//!             source
//! source = availability u32, [artifact u64, start u64, length u64, display-path string, line u64]
//! source-file = kind, artifact u64, byte-length u64
//! module = kind, compiler-module string, expected-bitcode-path string
//! string = byte-length u32, UTF-8 bytes
//! ```
//!
//! Booleans use zero or one. The named source-availability, LTO, and LLVM-support constants define
//! the other field codes. Only [`SOURCE_AVAILABLE`] includes the bracketed source fields.
//!
//! Source-file IDs start at zero and increase by one for each new compiler source-file identity, in
//! first-capture order. Each ID is a little-endian `u64`, unique only within its attempt. Source
//! files use `artifact-<16-lowercase-hex-id>` in that directory. [`PROTOCOL_VERSION`] covers this
//! ID encoding and the record layout independently of durable capture IDs.
//!
//! Module paths come from rustc's output naming API and must name immediate files in the attempt's
//! `llvm` directory. The parent converts every expected module with the matching sysroot's
//! disassembler. No module record is valid for an unsupported configuration. File and module
//! identities must be unique.
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

/// Includes a whole-item range in a compiler-loaded source snapshot.
pub(crate) const SOURCE_AVAILABLE: u32 = 0;
/// Identifies a definition outside the selected crate.
pub(crate) const SOURCE_NONLOCAL: u32 = 1;
/// Indicates that the compiler has no loaded source text or local path.
pub(crate) const SOURCE_UNLOADED: u32 = 2;
/// Identifies generated, synthetic, or macro-expanded source.
pub(crate) const SOURCE_GENERATED: u32 = 3;
/// Indicates that the span cannot identify one valid local source range.
pub(crate) const SOURCE_UNSUPPORTED_SPAN: u32 = 4;
/// Identifies a source path outside the canonical selected package root.
pub(crate) const SOURCE_OUTSIDE_PACKAGE: u32 = 5;

/// Indicates that the compiler performs neither local nor cross-crate LTO.
pub(crate) const LTO_OFF: u32 = 0;
/// Identifies ThinLTO within the selected crate.
pub(crate) const LTO_LOCAL_THIN: u32 = 1;
/// Identifies ThinLTO across crates.
pub(crate) const LTO_CROSS_CRATE_THIN: u32 = 2;
/// Identifies fat LTO.
pub(crate) const LTO_FAT: u32 = 3;

/// Selects the verified optimized-LLVM collection recipe.
pub(crate) const LLVM_SUPPORTED: u32 = 0;
/// Excludes incremental compilation from optimized-LLVM collection.
pub(crate) const LLVM_UNSUPPORTED_INCREMENTAL: u32 = 1;
/// Excludes cross-crate ThinLTO from optimized-LLVM collection.
pub(crate) const LLVM_UNSUPPORTED_CROSS_CRATE_THIN: u32 = 2;
/// Excludes fat LTO from optimized-LLVM collection.
pub(crate) const LLVM_UNSUPPORTED_FAT: u32 = 3;
/// Excludes linker-plugin LTO from optimized-LLVM collection.
pub(crate) const LLVM_UNSUPPORTED_LINKER_PLUGIN: u32 = 4;
/// Identifies a backend other than LLVM.
pub(crate) const LLVM_UNSUPPORTED_OTHER_BACKEND: u32 = 5;
/// Identifies a compiler whose optimized-artifact mapping has not been verified.
pub(crate) const LLVM_UNSUPPORTED_UNVERIFIED_COMPILER: u32 = 6;

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
///
/// The marker **must** be the collector's fixed-length selected-target argument. A marker longer
/// than the protocol's `u32` string-length field causes a panic.
pub(crate) fn header(marker: &str) -> Vec<u8> {
    let mut bytes = MANIFEST_MAGIC.to_vec();
    bytes.extend(PROTOCOL_VERSION.to_le_bytes());

    bytes.extend(
        u32::try_from(marker.len())
            .expect("the collector builds the marker from one fixed-length analysis token")
            .to_le_bytes(),
    );

    bytes.extend(marker.as_bytes());

    bytes
}
