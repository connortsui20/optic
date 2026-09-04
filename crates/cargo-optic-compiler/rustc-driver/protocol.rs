//! Defines the private protocol shared by the compiler crate and standalone driver.
//!
//! The compiler crate copies this file beside the driver source before compiling that driver. It
//! contains only constants so the two separately compiled programs cannot drift on their wire
//! format, defensive bounds, or private environment contract.

pub(crate) const MANIFEST_MAGIC: &[u8; 16] = b"CARGO_OPTIC_2\0\0\0";
pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const END_RECORD: u32 = 0;
pub(crate) const PLACEMENT_RECORD: u32 = 1;

// The child process is trusted to run compiler code, but its output can still be partial or corrupt
// after a compiler failure. These deliberately permissive MVP ceilings are corruption guards, not
// supported-workload limits. Raising them increases worst-case allocation and processing; lowering
// them can reject a valid large build. `MAX_MANIFEST_BYTES` normally binds first because each
// placement repeats its identity strings; the count ceilings remain independent guards for compact
// or malformed data.
pub(crate) const MAX_MANIFEST_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_INSTANCES: usize = 1_000_000;
pub(crate) const MAX_PLACEMENTS: usize = 4_000_000;
pub(crate) const MAX_STRING_BYTES: usize = 1024 * 1024;

pub(crate) const MANIFEST_PATH_ENV: &str = "OPTIC_COMPILER_MANIFEST";
pub(crate) const ORIGINAL_WRAPPER_ENV: &str = "OPTIC_ORIGINAL_RUSTC_WRAPPER";
pub(crate) const RUSTC_COMMAND_ENV: &str = "OPTIC_EXPECTED_RUSTC_COMMAND";
pub(crate) const RUSTC_COMMIT_ENV: &str = "OPTIC_EXPECTED_RUSTC_COMMIT";
pub(crate) const RUSTC_HOST_ENV: &str = "OPTIC_EXPECTED_RUSTC_HOST";
pub(crate) const RUSTC_PATH_ENV: &str = "OPTIC_EXPECTED_RUSTC_PATH";
pub(crate) const RUSTC_RELEASE_ENV: &str = "OPTIC_EXPECTED_RUSTC_RELEASE";
pub(crate) const RUSTC_SYSROOT_ENV: &str = "OPTIC_EXPECTED_RUSTC_SYSROOT";
pub(crate) const SELECTED_TARGET_MARKER_ENV: &str = "OPTIC_SELECTED_TARGET_MARKER";
pub(crate) const WORKSPACE_WRAPPER_ENV: &str = "OPTIC_HAS_WORKSPACE_WRAPPER";
pub(crate) const WRAPPER_ACTIVE_ENV: &str = "OPTIC_RUSTC_WRAPPER_ACTIVE";
pub(crate) const DRIVER_INNER_ENV: &str = "OPTIC_RUSTC_DRIVER_INNER";
