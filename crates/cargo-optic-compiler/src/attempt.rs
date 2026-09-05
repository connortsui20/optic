//! Runs one Cargo invocation and retains its diagnostics until classification.
//!
//! Stderr goes to an attempt-local file while stdout yields structured messages. A stopped probe
//! retains this file until collection succeeds or fails, so an intentional abort is not replayed
//! after a successful retry.

use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;

use cargo_metadata::Message;
use cargo_metadata::PackageId;
use cargo_metadata::Target;
use cargo_metadata::diagnostic::DiagnosticLevel;

use crate::Error;
use crate::observation::CargoObservation;

pub(crate) struct CargoAttempt {
    /// Owns the stderr file, manifest, and stale receipt until the attempt is no longer needed.
    temporary: tempfile::TempDir,
    /// Cargo's process result, independent of the structured build-finished message.
    pub(crate) status: ExitStatus,
    /// Selected artifacts and structured completion facts.
    pub(crate) observation: CargoObservation,
    diagnostics: Vec<String>,
    warnings: Vec<String>,
}

impl CargoAttempt {
    pub(crate) fn run(
        mut command: Command,
        temporary: tempfile::TempDir,
        package: &PackageId,
        target: &Target,
    ) -> Result<Self, Error> {
        let path = temporary.path().join("stderr");
        let stderr = File::create(&path).map_err(|source| Error::Filesystem {
            operation: "create Cargo diagnostic file",
            path: path.clone(),
            source,
        })?;
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(stderr);
        let program = command.get_program().to_owned();
        let mut child = command.spawn().map_err(|source| Error::StartProcess {
            program: program.clone().into(),
            source,
        })?;
        let stdout = child
            .stdout
            .take()
            .expect("the Cargo command configures piped stdout");
        let mut observation = CargoObservation::default();
        let mut diagnostics = Vec::new();
        let mut warnings = Vec::new();
        let mut read_error = None;

        for message in Message::parse_stream(BufReader::new(stdout)) {
            match message {
                Ok(Message::CompilerArtifact(artifact)) => {
                    observation.record(artifact, package, target)
                }
                Ok(Message::BuildFinished(finished)) => observation.finished.push(finished.success),
                Ok(Message::CompilerMessage(message)) => {
                    let diagnostic = message.message;
                    let rendered = diagnostic
                        .rendered
                        .unwrap_or_else(|| format!("{}\n", diagnostic.message));
                    if diagnostic.level == DiagnosticLevel::Warning {
                        warnings.push(rendered);
                    } else {
                        observation.has_errors |= matches!(
                            diagnostic.level,
                            DiagnosticLevel::Error
                                | DiagnosticLevel::Ice
                                | DiagnosticLevel::FailureNote
                        );
                        diagnostics.push(rendered);
                    }
                }
                Ok(Message::TextLine(line)) if !line.is_empty() => {
                    diagnostics.push(format!("{line}\n"))
                }
                Ok(_) => {}
                Err(error) => {
                    read_error.get_or_insert(error);
                }
            }
        }

        let status = child.wait().map_err(|source| Error::Filesystem {
            operation: "wait for Cargo process",
            path: program.into(),
            source,
        })?;
        let mut attempt = Self {
            temporary,
            status,
            observation,
            diagnostics,
            warnings,
        };
        if let Some(source) = read_error {
            attempt.replay()?;

            return Err(Error::Filesystem {
                operation: "read Cargo messages for",
                path,
                source,
            });
        }

        Ok(attempt)
    }

    pub(crate) fn path(&self, name: &str) -> std::path::PathBuf {
        self.temporary.path().join(name)
    }

    pub(crate) fn relay_warnings(&mut self) -> Result<(), Error> {
        for warning in &self.warnings {
            write_diagnostic(warning.as_bytes())?;
        }
        self.warnings.clear();

        Ok(())
    }

    pub(crate) fn replay(&mut self) -> Result<(), Error> {
        self.relay_warnings()?;
        for diagnostic in &self.diagnostics {
            write_diagnostic(diagnostic.as_bytes())?;
        }
        let path = self.path("stderr");
        let mut file = File::open(&path).map_err(|source| Error::Filesystem {
            operation: "open Cargo diagnostic file",
            path: path.clone(),
            source,
        })?;
        io::copy(&mut file, &mut io::stderr().lock()).map_err(|source| Error::Filesystem {
            operation: "replay Cargo diagnostics from",
            path,
            source,
        })?;

        Ok(())
    }

    pub(crate) fn failure(&self, cargo: &Path) -> Error {
        Error::ProcessFailed {
            program: cargo.to_owned(),
            status: self.status.to_string(),
            diagnostics: None,
        }
    }
}

fn write_diagnostic(bytes: &[u8]) -> Result<(), Error> {
    io::stderr()
        .lock()
        .write_all(bytes)
        .map_err(|source| Error::Filesystem {
            operation: "write Cargo diagnostics to",
            path: "stderr".into(),
            source,
        })
}
