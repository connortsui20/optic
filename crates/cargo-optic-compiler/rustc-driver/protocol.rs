//! Defines the private protocol shared by the compiler crate and standalone driver.

pub(crate) const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_2\0\0\0";
pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const END_RECORD: u32 = 0;
pub(crate) const PLACEMENT_RECORD: u32 = 1;

pub(crate) const MANIFEST_PATH_ENV: &str = "OPTIC_COMPILER_MANIFEST";
pub(crate) const SELECTED_TARGET_MARKER_ENV: &str = "OPTIC_SELECTED_TARGET_MARKER";
pub(crate) const DRIVER_INNER_ENV: &str = "OPTIC_RUSTC_DRIVER_INNER";
