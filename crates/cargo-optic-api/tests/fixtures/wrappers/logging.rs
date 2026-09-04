use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;
use std::process::exit;

fn main() {
    let executable = env::current_exe().expect("the wrapper path is available");
    let marker = executable.with_extension("marker");
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(marker)
            .expect("the marker can be opened"),
        "invoked",
    )
    .expect("the marker can be written");

    let mut arguments = env::args_os().skip(1);
    let program = arguments.next().expect("the wrapped program is present");
    let status = Command::new(program)
        .args(arguments)
        .status()
        .expect("the wrapped program can run");

    exit(status.code().unwrap_or(1));
}
