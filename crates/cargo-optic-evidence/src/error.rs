//! Describes invalid searches and failures while reading capture evidence.
//!
//! Query validation remains distinct from store failures so an application can report whether the
//! user must change the search or repair the selected capture.

use snafu::Snafu;

/// Explains why a stored evidence query could not complete.
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
    /// The result limit was zero.
    #[snafu(display("instance result limit must be at least 1, got {actual}"))]
    InvalidLimit {
        /// The rejected limit.
        actual: usize,
    },
    /// The reference selects a position outside the capture's immutable instance list.
    #[snafu(display(
        "instance ordinal must be less than {instance_count} for its capture, got {reference}"
    ))]
    InvalidInstanceReference {
        /// The rejected capture-scoped reference.
        reference: optic_records::InstanceRef,
        /// The number of instances in the selected capture.
        instance_count: usize,
    },
    /// An exact direct-alias chain revisited a symbol in the same module.
    #[snafu(display(
        "LLVM alias chain for {reference} in module {compiler_module:?} must be acyclic, \
         got repeated symbol {raw_symbol:?}"
    ))]
    AliasCycle {
        /// The instance whose raw symbol started the chain.
        reference: optic_records::InstanceRef,
        /// The compiler identity of the module that contains the cycle.
        compiler_module: String,
        /// The exact symbol that the chain visited twice.
        raw_symbol: String,
    },
    /// The selected capture's evidence could not be read.
    #[snafu(display("failed to read stored evidence"))]
    Store {
        /// The underlying store failure.
        source: optic_store::Error,
    },
}
