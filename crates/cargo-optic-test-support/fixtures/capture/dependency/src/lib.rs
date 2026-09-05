//! Supplies a tracked local dependency for capture tests.
//!
//! The fixture has no registry dependencies and builds offline.

/// Returns the dependency's observable value.
pub fn value() -> u64 {
    42
}
