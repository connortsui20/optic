//! Provides a feature-selected function for source and output-stream acceptance tests.
//!
//! The application instantiates this generic function only when its optional feature is enabled.
//! The deprecation warning supplies one rendered compiler diagnostic for the output observer.

/// Returns one value from a feature-selected local package.
///
/// The acceptance test prevents inlining so that the selected target retains a concrete instance.
#[deprecated(note = "streamed fixture warning")]
#[inline(never)]
pub fn optional_source<T>(value: T) -> T {
    std::hint::black_box(value)
}
