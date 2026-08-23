//! Dispatches parsed commands through the Cargo Optic application API.
//!
//! [`crate::arguments`] owns command-line grammar and validation. This module opens the applicable
//! workspace, calls [`Optic`], and sends returned records to [`CaptureOutput`]. It does not inspect
//! Cargo metadata, storage paths, or record fields directly.

use std::env;

use optic::Optic;
use snafu::ResultExt;
use snafu::Snafu;

use crate::arguments;
use crate::arguments::Command;
use crate::output::CaptureOutput;

pub(crate) fn run() -> Result<(), Error> {
    let command = arguments::parse()?;
    let directory = env::current_dir().context(CurrentDirectorySnafu)?;
    let optic = Optic::open(&directory)?;

    match command {
        Command::Capture(request) => {
            let capture = optic.capture(&request)?;
            let output = CaptureOutput::new("Captured", &capture)?;

            print!("{output}");
        }
        Command::Captures => {
            let captures = optic.captures()?;
            if captures.is_empty() {
                println!("No captures.");

                return Ok(());
            }

            println!("Captures");
            for capture in captures {
                let output = CaptureOutput::new("Capture", &capture)?;

                println!();
                print!("{output}");
            }
        }
    }

    Ok(())
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("failed to read the current directory"))]
    CurrentDirectory { source: std::io::Error },

    #[snafu(transparent)]
    Product { source: optic::Error },

    #[snafu(transparent)]
    InvalidBuildRequest { source: optic::InvalidBuildRequest },

    #[snafu(transparent)]
    Output { source: crate::output::Error },
}
