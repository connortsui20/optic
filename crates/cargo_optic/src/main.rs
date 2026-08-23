//! Provides the `cargo optic` command-line application.
//!
//! [`arguments`] owns the command grammar, [`cli`] dispatches to the [`optic`] application API, and
//! [`output`] owns the human-readable view of returned records.
//!
//! The binary contains no compiler or persistence logic. Keeping those boundaries behind [`optic`]
//! gives other frontends the same capture semantics without coupling them to Clap or terminal text.
//! SNAFU's [`snafu::Report`] renders the complete error chain at this final application boundary.

mod arguments;
mod cli;
mod output;

fn main() -> snafu::Report<cli::Error> {
    cli::run().into()
}
