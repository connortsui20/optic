//! Provides the `cargo optic` command-line application.
//!
//! [`arguments`] owns the command grammar, [`cli`] dispatches to the [`optic`] application API, and
//! [`output`] owns the human-readable view of returned records.
//!
//! Compiler and persistence logic remain behind [`optic`], so other front-ends can use the same
//! capture semantics without depending on Clap or terminal output. [`snafu::Report`] renders the
//! complete error chain at this final application boundary.

mod arguments;
mod cli;
mod output;

fn main() -> snafu::Report<cli::Error> {
    cli::run().into()
}
