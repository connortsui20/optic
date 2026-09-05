//! Defines the private protocol shared by the compiler crate and standalone driver.
//!
//! The manifest is a little-endian stream with this shape:
//!
//! ```text
//! magic, version, selected-marker string, placement*, end
//! placement = kind, four instance strings, three placement strings, local-copy u32, size u64
//! string = byte-length u32, UTF-8 bytes
//! ```
//!
//! This protocol is private because both ends ship in the same Cargo Optic binary. Its version is
//! independent of the durable capture format. A stale receipt contains only the header. Its marker
//! binds the stopped invocation to the candidate token. Manifest readers require the same marker.

/// Identifies a file as a Cargo Optic compiler manifest before any lengths are decoded.
pub(crate) const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_1\0\0\0";
/// Identifies the header that binds a manifest or stale receipt to an analysis marker.
pub(crate) const PROTOCOL_VERSION: u32 = 2;
/// Marks the successful end of the record stream.
pub(crate) const END_RECORD: u32 = 0;
/// Marks a concrete-instance placement record.
pub(crate) const PLACEMENT_RECORD: u32 = 1;

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
