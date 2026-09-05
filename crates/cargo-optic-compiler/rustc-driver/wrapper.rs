//! Implements Cargo's outer rustc-wrapper calling convention.
//!
//! Cargo passes the real compiler as argument zero. Only the selected target has the marker that
//! Cargo Optic appended after `cargo rustc --`; every other invocation is forwarded unchanged.

use std::env;
use std::process::Command;
use std::process::ExitCode;

use crate::failure;
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
    if let Some(index) = selected_target {
        arguments.remove(index);
        let executable = match env::current_exe() {
            Ok(executable) => executable,
            Err(error) => return failure(format!("failed to find the rustc driver: {error}")),
        };
        arguments.insert(0, executable.into_os_string());
    }

    let program = arguments.remove(0);
    let mut command = Command::new(program);
    command.args(arguments);
    if selected_target.is_some() {
        command.env(DRIVER_INNER_ENV, "1");
    }

    execute(command)
}

/// Replaces the wrapper process so rustc receives signals and exit handling directly from Cargo.
fn execute(mut command: Command) -> ExitCode {
    use std::os::unix::process::CommandExt;

    let error = command.exec();

    failure(format!("failed to start rustc: {error}"))
}
