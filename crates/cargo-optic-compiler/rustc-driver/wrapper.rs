//! Implements Cargo's outer rustc-wrapper calling convention.
//!
//! Cargo passes the real compiler as argument zero. Discovery and nonselected compilation forward
//! to that compiler. A selected invocation must match the prepared compiler, source, and crate name.
//! Probe mode writes a receipt and stops before analysis. Collection removes the marker and starts
//! the inner driver with the original Rust arguments.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::ExitCode;

use crate::failure;
use crate::protocol;
use crate::protocol::DRIVER_INNER_ENV;
use crate::protocol::SELECTED_TARGET_MARKER_ENV;

/// Replaces the wrapper with rustc or with the inner analysis invocation.
pub(crate) fn run() -> ExitCode {
    let mut arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() {
        return failure("optic rustc wrapper must receive a compiler path, got none");
    }

    let Some(marker) = env::var_os(SELECTED_TARGET_MARKER_ENV) else {
        return failure(format!("{SELECTED_TARGET_MARKER_ENV} is not set"));
    };
    let selected_target = arguments.iter().position(|argument| argument == &marker);
    if selected_target.is_none() {
        if arguments.iter().any(|argument| {
            argument
                .as_encoded_bytes()
                .starts_with(protocol::MARKER_PREFIX.as_bytes())
        }) {
            return failure(
                "selected invocation must use this analysis marker, got another marker",
            );
        }

        let program = arguments.remove(0);
        let mut command = Command::new(program);
        command.args(arguments);

        return execute(command);
    }

    if let Err(error) = verify_selected(&arguments, &marker) {
        return failure(error);
    }

    match env::var(protocol::MODE_ENV).as_deref() {
        Ok("probe") => {
            let Some(path) = env::var_os(protocol::STALE_RECEIPT_ENV) else {
                return failure(format!("{} is not set", protocol::STALE_RECEIPT_ENV));
            };
            let Some(marker) = marker.to_str() else {
                return failure("selected marker must be UTF-8, got a non-UTF-8 argument");
            };
            if let Err(error) = fs::write(path, protocol::header(marker)) {
                return failure(format!("failed to write stale receipt: {error}"));
            }

            return failure("Cargo Optic stopped a stale selected-target probe before analysis");
        }
        Ok("collect") => {}
        _ => return failure("compiler mode must be probe or collect, got an unsupported value"),
    }

    arguments.remove(selected_target.expect("the forwarding branch returned for an absent marker"));
    let executable = match env::current_exe() {
        Ok(executable) => executable,
        Err(error) => return failure(format!("failed to find the rustc driver: {error}")),
    };
    let mut command = Command::new(executable);
    command.args(arguments).env(DRIVER_INNER_ENV, "1");

    execute(command)
}

fn verify_selected(arguments: &[OsString], marker: &OsString) -> Result<(), String> {
    let compiler = env::var_os(protocol::RUSTC_ENV).ok_or("selected compiler is not set")?;
    let source = env::var_os(protocol::SOURCE_ENV).ok_or("selected source is not set")?;
    let crate_name =
        env::var_os(protocol::CRATE_NAME_ENV).ok_or("selected crate name is not set")?;
    let markers = arguments
        .iter()
        .filter(|argument| {
            argument
                .as_encoded_bytes()
                .starts_with(protocol::MARKER_PREFIX.as_bytes())
        })
        .count();
    if markers != 1 || !arguments.contains(marker) {
        return Err(format!(
            "selected invocation requires one matching marker, got {markers} markers"
        ));
    }
    if arguments[0] != compiler {
        return Err(format!(
            "selected invocation requires the prepared compiler, got {:?}",
            arguments[0]
        ));
    }
    let actual_name = arguments
        .windows(2)
        .find(|pair| pair[0] == "--crate-name")
        .map(|pair| &pair[1]);
    if actual_name != Some(&crate_name) {
        return Err(format!(
            "selected invocation requires the prepared crate name, got {actual_name:?}"
        ));
    }
    let source_matches = arguments.iter().skip(1).any(|argument| {
        !argument.as_encoded_bytes().starts_with(b"-")
            && fs::canonicalize(argument).is_ok_and(|path| path == Path::new(&source))
    });
    if !source_matches {
        return Err(
            "selected invocation requires the prepared source path, got no matching source"
                .to_owned(),
        );
    }

    Ok(())
}

/// Replaces the wrapper process so rustc receives signals and exit handling directly from Cargo.
fn execute(mut command: Command) -> ExitCode {
    use std::os::unix::process::CommandExt;

    let error = command.exec();

    failure(format!("failed to start rustc: {error}"))
}
