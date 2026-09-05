//! Exercises Unicode positions and a source path that contains spaces.
//!
//! Source tests convert this file to BOM/CRLF before the compiler loads it.

const LABEL: &str = "π 雪 🦀";

#[inline(never)]
/// Keeps Unicode text before the function's byte range.
pub fn unicode_function(value: u64) -> u64 {
    std::hint::black_box(LABEL);
    value.wrapping_add(11)
}
