//! Defines the private protocol shared by the compiler crate and standalone driver.
//!
//! The manifest is a little-endian stream with this shape:
//!
//! ```text
//! magic, version, placement*, end
//! placement = kind, four instance strings, three placement strings, local-copy u32, size u64
//! string = byte-length u32, UTF-8 bytes
//! ```
//!
//! This protocol is private because both ends ship in the same Cargo Optic binary. Its version is
//! independent of the durable capture format.

/// Identifies a file as a Cargo Optic compiler manifest before any lengths are decoded.
pub(crate) const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_1\0\0\0";
/// Identifies the first layout of the private compiler-manifest protocol.
pub(crate) const PROTOCOL_VERSION: u32 = 1;
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
