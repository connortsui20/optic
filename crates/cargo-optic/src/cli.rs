//! Dispatches parsed commands through the Cargo Optic application API.
//!
//! [`crate::arguments`] owns command-line grammar and validation. This module opens the applicable
//! workspace, calls [`Optic`], and writes returned records to caller-provided streams. It does not
//! inspect Cargo metadata, storage paths, or record fields directly.

use std::env;
use std::io;
use std::io::Write;

use optic::Optic;
use snafu::ResultExt;
use snafu::Snafu;

use crate::arguments;
use crate::output::CaptureOutput;
use crate::output::InstanceOutput;

pub(crate) fn run() -> Result<(), Error> {
    let stdout = io::stdout();

    match run_with_output(stdout.lock()) {
        Err(Error::Write { source, .. }) if source.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

fn run_with_output(mut stdout: impl Write) -> Result<(), Error> {
    let command = arguments::parse()?;

    let directory = env::current_dir().context(CurrentDirectorySnafu)?;
    let optic = Optic::open(&directory)?;

    match command {
        arguments::Command::Capture(request) => {
            let capture = optic.capture(&request)?;
            let output = CaptureOutput::new("Captured", &capture)?;

            write!(stdout, "{output}").context(WriteSnafu)?;
        }
        arguments::Command::ListCaptures => {
            let captures = optic.list_captures()?;
            if captures.is_empty() {
                writeln!(stdout, "No captures.").context(WriteSnafu)?;

                return Ok(());
            }

            writeln!(stdout, "Captures").context(WriteSnafu)?;
            for capture in captures {
                let output = CaptureOutput::new("Capture", &capture)?;

                writeln!(stdout).context(WriteSnafu)?;
                write!(stdout, "{output}").context(WriteSnafu)?;
            }
        }
        arguments::Command::Find {
            capture,
            query,
            limit,
        } => {
            let results = optic.find(&capture, &query, limit)?;
            if results.instances().is_empty() {
                writeln!(stdout, "No instances found.").context(WriteSnafu)?;

                return Ok(());
            }

            for (index, instance) in results.instances().iter().enumerate() {
                if index != 0 {
                    writeln!(stdout).context(WriteSnafu)?;
                }

                let output = InstanceOutput::new(results.capture_id(), instance);
                write!(stdout, "{output}").context(WriteSnafu)?;
            }

            if results.is_truncated() {
                writeln!(stdout).context(WriteSnafu)?;
                writeln!(
                    stdout,
                    "Showing {} of {} matching instances. Narrow the query to reduce the result set.",
                    results.instances().len(),
                    results.total_matches(),
                )
                .context(WriteSnafu)?;
            }
        }
    }

    Ok(())
}

/// Explains why the command-line application could not complete an operation.
#[derive(Debug, Snafu)]
pub(crate) enum Error {
    /// The process could not read its invocation directory.
    #[snafu(display("failed to read the current directory"))]
    CurrentDirectory {
        /// The current-directory error.
        source: io::Error,
    },

    /// The application could not write its human-readable output.
    #[snafu(display("failed to write stdout"))]
    Write {
        /// The standard-output error.
        source: io::Error,
    },

    /// The product API operation failed.
    #[snafu(transparent)]
    Product {
        /// The product API error.
        source: optic::Error,
    },

    /// The parsed build selectors did not form a valid request.
    #[snafu(transparent)]
    InvalidBuildRequest {
        /// The invalid request error.
        source: optic::InvalidBuildRequest,
    },

    /// A capture record could not become human-readable output.
    #[snafu(transparent)]
    Output {
        /// The output adapter error.
        source: crate::output::Error,
    },
}
