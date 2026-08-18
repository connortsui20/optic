//! Runs one enriched Cargo analysis and collects supported LLVM modules.
//!
//! [`capture`] uses `cargo rustc` so normal and analysis builds share dependency artifacts. The
//! selected target has a separate Cargo identity because its extra compiler arguments are part of
//! Cargo's fingerprint.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::llvm;
use crate::mono;
use crate::{BuildRequest, CargoTarget, Error, MonoItem, Result, Toolchain, inspect_toolchain};

/// The byte range of one LLVM function definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BodyRange {
    /// The raw LLVM symbol without its leading `@`.
    pub raw_symbol: String,

    /// The best available demangled display name.
    pub demangled: String,

    /// The inclusive byte offset at which the definition starts.
    pub start: u64,

    /// The exclusive byte offset at which the definition ends.
    pub end: u64,
}

/// One disassembled LLVM module and its body index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleEvidence {
    /// The compiler-owned artifact file name.
    pub name: String,

    /// The supported compiler stage.
    pub stage: String,

    /// The saved LLVM bitcode path.
    pub bitcode_path: PathBuf,

    /// The matching textual LLVM module path.
    pub text_path: PathBuf,

    /// Indexed function definitions in the textual module.
    pub bodies: Vec<BodyRange>,
}

/// All evidence produced by one compiler invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    /// The exact analyzed compiler.
    pub toolchain: Toolchain,

    /// Concrete functions selected by rustc.
    pub mono_items: Vec<MonoItem>,

    /// Supported saved LLVM modules.
    pub modules: Vec<ModuleEvidence>,
}

/// The result of asking Cargo for one analysis unit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaptureOutcome {
    /// Cargo reported the analysis unit as fresh and produced no new evidence.
    Fresh {
        /// The exact active compiler.
        toolchain: Toolchain,
    },

    /// Cargo compiled the selected unit and produced evidence.
    Captured(EvidenceBundle),
}

/// Runs one enriched analysis for the selected Cargo target.
pub fn capture(request: &BuildRequest) -> Result<CaptureOutcome> {
    let toolchain = inspect_toolchain()?;
    prepare_analysis_directory(&request.analysis_directory)?;

    let output = cargo_command(request)
        .output()
        .map_err(|source| Error::StartProcess {
            program: "cargo rustc".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: "cargo rustc".to_owned(),
            status: output.status.to_string(),
            diagnostics: cargo_diagnostics(&output.stdout, &output.stderr),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if selected_target_is_fresh(&output.stdout) {
        return Ok(CaptureOutcome::Fresh { toolchain });
    }

    let artifacts = supported_bitcode(&request.analysis_directory)?;

    if artifacts.is_empty() {
        return Ok(CaptureOutcome::Fresh { toolchain });
    }

    let mono_output = format!("{stdout}\n{stderr}");
    let mono_items = mono::parse(&mono_output);
    let modules = artifacts
        .into_iter()
        .map(|(bitcode_path, stage)| {
            disassemble(&toolchain, bitcode_path, stage, &request.analysis_directory)
        })
        .collect::<Result<Vec<_>>>()?;

    if modules.is_empty() {
        return Err(Error::MissingEvidence);
    }

    Ok(CaptureOutcome::Captured(EvidenceBundle {
        toolchain,
        mono_items,
        modules,
    }))
}

fn cargo_command(request: &BuildRequest) -> Command {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command.current_dir(&request.workspace_root);
    command.arg("rustc");
    command.arg("--message-format=json-render-diagnostics");

    if let Some(path) = &request.manifest_path {
        command.arg("--manifest-path").arg(path);
    }
    if let Some(package) = &request.package {
        command.arg("--package").arg(package);
    }
    if let Some(target) = &request.target {
        match target {
            CargoTarget::Library => {
                command.arg("--lib");
            }
            CargoTarget::Binary(name) => {
                command.arg("--bin").arg(name);
            }
            CargoTarget::Benchmark(name) => {
                command.arg("--bench").arg(name);
            }
            CargoTarget::Example(name) => {
                command.arg("--example").arg(name);
            }
        }
    }
    if let Some(profile) = &request.profile {
        command.arg("--profile").arg(profile);
    }
    if !request.features.is_empty() {
        command.arg("--features").arg(request.features.join(","));
    }
    if request.all_features {
        command.arg("--all-features");
    }
    if request.no_default_features {
        command.arg("--no-default-features");
    }
    if let Some(target) = &request.target_triple {
        command.arg("--target").arg(target);
    }
    if request.locked {
        command.arg("--locked");
    }
    if request.offline {
        command.arg("--offline");
    }
    if request.frozen {
        command.arg("--frozen");
    }

    command.arg("--");
    command.args(["-Z", "no-link", "-C", "save-temps"]);
    command.args(["-Z", "print-mono-items=yes"]);
    command
        .arg("-Z")
        .arg(temps_argument(&request.analysis_directory));
    command.args(["-C", "symbol-mangling-version=v0"]);
    command.args(["-C", "debuginfo=line-tables-only"]);

    command
}

fn temps_argument(path: &Path) -> OsString {
    let mut argument = OsString::from("temps-dir=");
    argument.push(path);
    argument
}

fn prepare_analysis_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::Filesystem {
        operation: "create",
        path: path.to_owned(),
        source,
    })
}

fn selected_target_is_fresh(stdout: &[u8]) -> bool {
    stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter_map(|message| message["fresh"].as_bool())
        .next_back()
        .unwrap_or(false)
}

fn cargo_diagnostics(stdout: &[u8], stderr: &[u8]) -> String {
    let mut diagnostics = String::new();

    for message in stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
    {
        let Some(rendered) = message["message"]["rendered"].as_str() else {
            continue;
        };

        diagnostics.push_str(rendered);

        if !rendered.ends_with('\n') {
            diagnostics.push('\n');
        }
    }

    diagnostics.push_str(&String::from_utf8_lossy(stderr));

    if diagnostics.is_empty() {
        diagnostics.push_str(&String::from_utf8_lossy(stdout));
    }

    diagnostics
}

fn supported_bitcode(directory: &Path) -> Result<Vec<(PathBuf, &'static str)>> {
    let mut artifacts = Vec::new();

    for entry in WalkDir::new(directory).min_depth(1) {
        let entry = entry.map_err(|source| Error::Filesystem {
            operation: "walk",
            path: directory.to_owned(),
            source: source.into(),
        })?;

        if !entry.file_type().is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy();
        let stage = if name.ends_with(".no-opt.bc") {
            Some("llvm-pre-optimization")
        } else if name.ends_with(".rcgu.bc") && !name.contains(".thin-lto-") {
            Some("llvm-optimized")
        } else {
            None
        };

        if let Some(stage) = stage {
            artifacts.push((entry.into_path(), stage));
        }
    }

    artifacts.sort_by(|left, right| left.0.cmp(&right.0));

    Ok(artifacts)
}

fn disassemble(
    toolchain: &Toolchain,
    bitcode_path: PathBuf,
    stage: &'static str,
    analysis_directory: &Path,
) -> Result<ModuleEvidence> {
    let file_name = bitcode_path
        .file_name()
        .map_or_else(|| "module.bc".into(), |name| name.to_owned());
    let mut text_name = file_name;
    text_name.push(".ll");
    let text_path = analysis_directory.join(text_name);
    let output = Command::new(&toolchain.llvm_dis)
        .arg("-o")
        .arg(&text_path)
        .arg(&bitcode_path)
        .output()
        .map_err(|source| Error::StartProcess {
            program: toolchain.llvm_dis.display().to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: toolchain.llvm_dis.display().to_string(),
            status: output.status.to_string(),
            diagnostics: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let bodies = llvm::scan(&text_path)?;
    let name = bitcode_path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());

    Ok(ModuleEvidence {
        name,
        stage: stage.to_owned(),
        bitcode_path,
        text_path,
        bodies,
    })
}

#[cfg(test)]
mod tests {
    use super::cargo_diagnostics;

    #[test]
    fn renders_cargo_json_diagnostics_and_standard_error() {
        let stdout = br#"{"reason":"compiler-message","message":{"rendered":"error: bad input\n"}}
{"reason":"build-finished","success":false}
"#;

        assert_eq!(
            cargo_diagnostics(stdout, b"error: could not compile\n"),
            "error: bad input\nerror: could not compile\n"
        );
    }
}
