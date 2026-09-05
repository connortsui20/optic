//! Provides an ordinary function with a local dependency for capture tests.
//!
//! Changes to the dependency remain visible in the selected package's compilation.

/// Returns the value supplied by the local dependency.
pub fn captured_value() -> u64 {
    fixture_dependency::value()
}
