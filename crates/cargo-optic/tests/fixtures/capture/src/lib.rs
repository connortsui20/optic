//! Provides a library target for command integration tests.
//!
//! The target exists so tests can observe a completed library build.

pub fn captured_value() -> u64 {
    42
}
