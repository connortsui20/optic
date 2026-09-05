//! Keeps collection, record validation, and publication failures distinct.
//!
//! The capture workflow crosses several subsystem boundaries. Typed sources preserve the phase
//! that failed without duplicating their error contracts here.

use snafu::Snafu;

/// Explains why a complete capture could not be published.
#[derive(Debug, Snafu)]
#[non_exhaustive]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// Compiler collection did not produce validated selected-target evidence.
    #[snafu(transparent)]
    Compiler {
        /// The compiler subsystem error.
        source: optic_compiler::Error,
    },

    /// Collected evidence could not become a valid durable record.
    #[snafu(transparent)]
    Record {
        /// The record validation error.
        source: optic_records::Error,
    },

    /// The complete capture could not be published.
    #[snafu(transparent)]
    Store {
        /// The store subsystem error.
        source: optic_store::Error,
    },
}
