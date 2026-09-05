//! Supplies a root package with Cargo's default build-script file tracking.
//!
//! Tests initialize Git explicitly when they need Git's package-file selection.

/// Returns one observable value from the selected library.
pub fn captured_value() -> u64 {
    42
}
