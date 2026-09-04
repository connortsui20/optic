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

    /// The system clock was earlier than the Unix epoch.
    #[snafu(display("system clock must be at or after the Unix epoch"))]
    Clock {
        /// The invalid system time.
        source: std::time::SystemTimeError,
    },

    /// The completion timestamp did not fit in the durable record field.
    #[snafu(display("capture timestamp must fit in u64 milliseconds, got {actual}"))]
    TimestampOverflow {
        /// The overflowing millisecond count.
        actual: u128,
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
