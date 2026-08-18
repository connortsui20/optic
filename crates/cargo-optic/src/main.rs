//! Starts the Cargo Optic command-line interface.
//!
//! The [`cargo_optic`] library owns argument handling, application workflows, and output.

fn main() -> std::process::ExitCode {
    cargo_optic::run_cli()
}
