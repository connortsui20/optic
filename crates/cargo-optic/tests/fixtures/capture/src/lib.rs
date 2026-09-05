//! Provides a library target for command integration tests.
//!
//! The target exists so tests can observe package compilation alongside selected binaries.

/// Returns one observable value from the library fixture.
pub fn captured_value() -> u64 {
    42
}
