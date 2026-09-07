//! Materializes every expected optimized module into immutable textual artifacts.
//!
//! The driver supplies regular codegen-unit identities and exact rustc-derived bitcode paths. This
//! module never scans output directories or infers modules from function placements. Unsupported
//! configurations retain source evidence without requesting any additional LLVM artifacts.

use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use optic_records::ArtifactId;
use optic_records::ArtifactKind;
use optic_records::ArtifactRecord;
use optic_records::CompilerIdentity;
use optic_records::InstanceRecord;
use optic_records::LlvmCollection;
use optic_records::LlvmLto;
use optic_records::LlvmModuleRecord;
use optic_records::LlvmProvenance;
use optic_records::LlvmStage;

use crate::Error;
use crate::error::invalid_environment;
use crate::manifest::CompilerManifest;
use crate::manifest::ExpectedModule;
use crate::protocol::LLVM_RECIPE_REVISION;

/// Retains manifest metadata after source checks and optimized-module materialization.
pub(crate) struct CollectedEvidence {
    /// Concrete instances with checked source relationships.
    pub(crate) instances: Vec<InstanceRecord>,
    /// Generated source and textual LLVM files retained in the attempt directory.
    pub(crate) artifacts: Vec<ArtifactRecord>,
    /// Effective compiler settings and the matching LLVM version.
    pub(crate) provenance: LlvmProvenance,
    /// Complete optimized modules or the reason no LLVM was requested.
    pub(crate) llvm: LlvmCollection,
}

pub(crate) fn collect(
    manifest: CompilerManifest,
    compiler: &CompilerIdentity,
    directory: &Path,
) -> Result<CollectedEvidence, Error> {
    let configuration = manifest.configuration;
    let version = if configuration.backend == "llvm" {
        let output = run(Command::new(compiler.rustc()).arg("-vV"))?;
        let text = String::from_utf8_lossy(&output.stdout);

        Some(
            text.lines()
                .find_map(|line| line.strip_prefix("LLVM version: "))
                .ok_or_else(|| {
                    invalid_environment(
                        "the LLVM backend requires an exact compiler LLVM version, got none",
                    )
                })?
                .to_owned(),
        )
    } else {
        None
    };

    let provenance = LlvmProvenance::new(
        configuration.backend,
        version,
        configuration.target,
        configuration.optimization,
        configuration.lto,
        configuration.incremental,
        configuration.linker_plugin,
        configuration.codegen_units,
        LLVM_RECIPE_REVISION,
    )?;
    let mut artifacts = manifest.artifacts;

    for artifact in &artifacts {
        let path = directory.join(artifact.file_name());
        let metadata = regular_file(&path)?;

        if metadata.len() != artifact.byte_len() {
            return Err(invalid_environment(format!(
                "source snapshot length must match the compiler manifest, got {}",
                path.display()
            )));
        }
    }

    if let Some(reason) = configuration.unsupported {
        return Ok(CollectedEvidence {
            instances: manifest.instances,
            artifacts,
            provenance,
            llvm: LlvmCollection::NotCaptured(reason),
        });
    }

    let modules = collect_modules(
        manifest.modules,
        compiler,
        directory,
        &provenance,
        &mut artifacts,
    )?;

    Ok(CollectedEvidence {
        instances: manifest.instances,
        artifacts,
        provenance,
        llvm: LlvmCollection::Collected(modules),
    })
}

fn collect_modules(
    expected: Vec<ExpectedModule>,
    compiler: &CompilerIdentity,
    directory: &Path,
    provenance: &LlvmProvenance,
    artifacts: &mut Vec<ArtifactRecord>,
) -> Result<Vec<LlvmModuleRecord>, Error> {
    if provenance.backend() != "llvm" || provenance.incremental() || provenance.linker_plugin_lto()
    {
        return Err(invalid_environment(
            "collected LLVM requires a supported effective configuration, got conflicting metadata",
        ));
    }

    let stage = match provenance.lto() {
        LlvmLto::Off => LlvmStage::NoLtoOptimized,
        LlvmLto::LocalThin => LlvmStage::LocalThinLtoPostPassManager,
        _ => {
            return Err(invalid_environment(
                "collected LLVM requires no LTO or local ThinLTO, got another mode",
            ));
        }
    };

    let disassembler = compiler
        .sysroot()
        .join("lib/rustlib")
        .join(compiler.host())
        .join("bin/llvm-dis");

    if !disassembler.is_file() {
        return Err(invalid_environment(format!(
            "optimized LLVM collection requires {}. \
             Install the matching component with rustup component add llvm-tools",
            disassembler.display()
        )));
    }

    let output = run(Command::new(&disassembler).arg("--version"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let actual = disassembler_version(&text);

    if actual != provenance.llvm_version() {
        return Err(invalid_environment(format!(
            "llvm-dis must match the compiler's LLVM version, got {actual:?}"
        )));
    }

    let mut modules = Vec::with_capacity(expected.len());
    let mut next_id = match artifacts.iter().map(|artifact| artifact.id().value()).max() {
        Some(id) => id.checked_add(1).ok_or_else(|| {
            invalid_environment("artifact IDs must leave space for LLVM modules, got u64::MAX")
        })?,
        None => 0,
    };

    for module in expected {
        if regular_file(&module.path)?.len() == 0 {
            return Err(invalid_environment(format!(
                "expected bitcode must not be empty, got {}",
                module.path.display()
            )));
        }

        let id = ArtifactId::new(next_id);
        next_id = next_id
            .checked_add(1)
            .ok_or_else(|| invalid_environment("artifact IDs must fit in u64, got overflow"))?;
        let path = directory.join(ArtifactRecord::new(id, ArtifactKind::Llvm, 0).file_name());

        run(Command::new(&disassembler)
            .arg(&module.path)
            .arg("-o")
            .arg(&path))
        .map_err(|error| Error::Filesystem {
            operation: "disassemble expected bitcode module",
            path: module.path.clone(),
            source: io::Error::other(error),
        })?;

        let length = regular_file(&path)?.len();
        let file = File::open(&path).map_err(|source| Error::Filesystem {
            operation: "open optimized LLVM artifact",
            path: path.clone(),
            source,
        })?;

        let definitions =
            crate::llvm_index::index(BufReader::new(file)).map_err(|source| Error::Filesystem {
                operation: "index optimized LLVM artifact",
                path: path.clone(),
                source,
            })?;

        modules.push(LlvmModuleRecord::new(id, module.name, stage, definitions)?);
        artifacts.push(ArtifactRecord::new(id, ArtifactKind::Llvm, length));
    }

    Ok(modules)
}

fn regular_file(path: &Path) -> Result<fs::Metadata, Error> {
    let metadata = fs::symlink_metadata(path).map_err(|source| Error::Filesystem {
        operation: "inspect compiler artifact",
        path: path.to_owned(),
        source,
    })?;

    if !metadata.is_file() {
        return Err(Error::Filesystem {
            operation: "read regular compiler artifact",
            path: path.to_owned(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact must be a regular file",
            ),
        });
    }

    Ok(metadata)
}

fn run(command: &mut Command) -> Result<std::process::Output, Error> {
    let program = PathBuf::from(command.get_program());
    let output = command.output().map_err(|source| Error::StartProcess {
        program: program.clone(),
        source,
    })?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program,
            status: output.status.to_string(),
            diagnostics: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }

    Ok(output)
}

/// Rust's llvm-tools append their release label to LLVM's numeric version.
fn disassembler_version(output: &str) -> Option<&str> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("LLVM version "))
        .and_then(|version| version.split('-').next())
}

#[cfg(test)]
mod tests {
    use super::disassembler_version;

    #[test]
    fn disassembler_version_separates_the_rust_distribution_suffix() {
        assert_eq!(disassembler_version("LLVM version 22.1.8"), Some("22.1.8"));
        assert_eq!(
            disassembler_version("  LLVM version 22.1.8-rust-1.98.1-stable\n"),
            Some("22.1.8")
        );
        assert_eq!(disassembler_version("a different executable"), None);
    }
}
