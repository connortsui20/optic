//! Describes invalid searches and failures while reading capture evidence.
//!
//! Query validation remains distinct from store failures so an application can report whether the
//! user must change the search or repair the selected capture.

use snafu::Snafu;

/// Explains why an instance search could not complete.
#[derive(Debug, Snafu)]
#[non_exhaustive]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// The query was empty.
    #[snafu(display("instance query must contain at least one character, got {query:?}"))]
    EmptyQuery {
        /// The rejected query.
        query: String,
    },
    /// The result limit was outside the supported range.
    #[snafu(display("instance result limit must be between 1 and {maximum}, got {actual}"))]
    InvalidLimit {
        /// The rejected limit.
        actual: usize,
        /// The largest accepted limit.
        maximum: usize,
    },
    /// The selected capture's evidence could not be read.
    #[snafu(display("failed to read instance evidence"))]
    Store {
        /// The underlying store failure.
        source: optic_store::Error,
    },
}
