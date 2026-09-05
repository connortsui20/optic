//! A module with a space in its path for dependency file parsing tests.

/// Returns a value defined in a path that requires Makefile escaping.
#[inline(never)]
pub fn spaced_value(value: u64) -> u64 {
    std::hint::black_box(value)
}
