//! Reads the durable compiler identity through Cargo's rustc invocation.
//!
//! The parser requires every field stored by [`ToolchainRecord`]. Inspection executes the complete
//! [`RustcInvocation`] with `-vV`, so a wrapper that substitutes another compiler reports that
//! compiler's release, commit, host, and LLVM version.

use std::path::Path;
use std::process::Command;

use optic_records::RustcInvocation;
use optic_records::ToolchainRecord;
use snafu::ResultExt;

use crate::Error;
use crate::MissingToolchainFieldSnafu;
use crate::ProcessFailedSnafu;
use crate::StartProcessSnafu;

pub(crate) fn inspect_rustc(
    invocation: RustcInvocation,
    workspace_root: &Path,
) -> Result<ToolchainRecord, Error> {
    let mut programs = invocation.programs();
    let program = programs
        .next()
        .expect("RustcInvocation construction requires at least the rustc program");
    let mut command = Command::new(program);
    command
        .current_dir(workspace_root)
        .args(programs)
        .arg("-vV");
    let output = command.output().with_context(|_| StartProcessSnafu {
        program: program.to_owned(),
    })?;
    if !output.status.success() {
        return ProcessFailedSnafu {
            program: program.to_owned(),
            status: output.status.to_string(),
        }
        .fail();
    }

    parse_rustc_verbose(invocation, &String::from_utf8_lossy(&output.stdout))
}

fn parse_rustc_verbose(
    invocation: RustcInvocation,
    output: &str,
) -> Result<ToolchainRecord, Error> {
    Ok(ToolchainRecord::new(
        invocation,
        rustc_field(output, "release")?,
        rustc_field(output, "commit-hash")?,
        rustc_field(output, "host")?,
        rustc_field(output, "LLVM version")?,
    )?)
}

fn rustc_field<'a>(output: &'a str, name: &'static str) -> Result<&'a str, Error> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MissingToolchainFieldSnafu { field: name }.build())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use optic_records::RustcInvocation;

    use super::parse_rustc_verbose;

    const RUSTC_VERBOSE: &str = "rustc 1.98.0\n\
release: 1.98.0\n\
commit-hash: 0123456789abcdef\n\
host: test-host\n\
LLVM version: 22.1.8\n";

    #[test]
    fn parses_a_complete_rustc_identity() {
        let invocation = RustcInvocation::new(PathBuf::from("rustc"), None, None)
            .expect("the fixture compiler invocation is valid");
        let toolchain = parse_rustc_verbose(invocation, RUSTC_VERBOSE)
            .expect("the rustc fixture contains every recorded identity field");

        assert_eq!(toolchain.release(), "1.98.0");
        assert_eq!(toolchain.commit_hash(), "0123456789abcdef");
        assert_eq!(toolchain.host(), "test-host");
        assert_eq!(toolchain.llvm_version(), "22.1.8");
    }
}
