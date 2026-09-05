//! Provides an external definition instantiated by the selected fixture binary.
//!
//! Its source lies outside the selected package's approved local definitions.

#[inline(never)]
/// Instantiates a nonlocal definition in the selected binary.
pub fn external_generic<T: Copy>(value: T) -> T {
    std::hint::black_box(value)
}
