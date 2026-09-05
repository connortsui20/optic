//! A separate codegen unit candidate for local ThinLTO tests.
//!
//! The main function calls this function without an inline attribute. This source shape lets the
//! fixture compare per-CGU optimization with the normal local ThinLTO pipeline.

/// Computes a value in a source module outside the main function.
#[unsafe(export_name = "optic_cross_cgu")]
pub extern "C" fn cross_cgu(value: u64) -> u64 {
    value.rotate_left(7) ^ 0x9e37_79b9_7f4a_7c15
}
