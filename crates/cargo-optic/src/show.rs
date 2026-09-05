//! Writes exact stored evidence without surrounding text on stdout.
//!
//! The application API owns reference resolution and availability. This module only selects the
//! requested form, reports its origin on stderr, and copies the returned finite ranges.

use std::io::Write;

use optic::EvidenceRange;
use optic::InstanceRef;
use optic::LlvmEvidence;
use optic::Optic;
use optic::SourceEvidence;

use crate::arguments::EvidenceOutput;
use crate::cli::Error;

pub(crate) fn run(
    optic: &Optic,
    instance: &InstanceRef,
    output: EvidenceOutput,
    stdout: &mut impl Write,
) -> Result<(), Error> {
    match output {
        EvidenceOutput::Source => show_source(optic, instance, stdout),
        EvidenceOutput::Llvm => show_llvm(optic, instance, stdout),
    }
}

fn show_source(
    optic: &Optic,
    instance: &InstanceRef,
    stdout: &mut impl Write,
) -> Result<(), Error> {
    match optic.source(instance)? {
        SourceEvidence::Available {
            evidence,
            display_path,
            starting_line,
        } => {
            eprintln!(
                "Source {}:{starting_line} ({instance})",
                display_path.display()
            );

            copy(optic, &evidence, stdout)
        }
        SourceEvidence::Unavailable(reason) => {
            Err(unavailable(instance, "source", reason.to_string()))
        }
    }
}

fn show_llvm(optic: &Optic, instance: &InstanceRef, stdout: &mut impl Write) -> Result<(), Error> {
    let bodies = match optic.llvm(instance)? {
        LlvmEvidence::Available(bodies) => bodies,
        LlvmEvidence::NotCaptured(reason) => {
            return Err(unavailable(instance, "LLVM", reason.to_string()));
        }
        LlvmEvidence::NoExactDefinition => {
            return Err(unavailable(
                instance,
                "LLVM",
                "no exact standalone definition in the captured modules",
            ));
        }
        LlvmEvidence::UnsupportedAlias => {
            return Err(unavailable(
                instance,
                "LLVM",
                "an expression alias or indirect function that narrow show cannot resolve",
            ));
        }
    };

    if bodies.len() > 1 {
        eprintln!("LLVM has {} exact bodies for {instance}", bodies.len());
    }

    for (index, body) in bodies.iter().enumerate() {
        eprintln!("LLVM {} ({})", body.compiler_module(), body.stage());
        if !body.aliases().is_empty() {
            eprintln!(
                "Alias {} -> {}",
                body.aliases().join(" -> "),
                body.raw_symbol()
            );
        }
        if index != 0 {
            stdout
                .write_all(b"\n")
                .map_err(|source| Error::Write { source })?;
        }

        copy(optic, body.evidence(), stdout)?;
    }

    Ok(())
}

/// Preserves the CLI's closed-stdout policy without treating artifact read failures as pipe closure.
fn copy(optic: &Optic, evidence: &EvidenceRange, stdout: &mut impl Write) -> Result<(), Error> {
    match optic.copy_evidence(evidence, stdout) {
        Err(optic::Error::Store {
            source: optic::StoreError::WriteEvidence { source, .. },
        }) => Err(Error::Write { source }),
        result => result.map_err(Error::from),
    }
}

fn unavailable(instance: &InstanceRef, output: &'static str, reason: impl Into<String>) -> Error {
    Error::EvidenceUnavailable {
        instance: instance.clone(),
        output,
        reason: reason.into(),
    }
}
