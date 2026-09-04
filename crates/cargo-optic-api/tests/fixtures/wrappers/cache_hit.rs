use std::env;
use std::path::Path;
use std::process::Command;
use std::process::exit;

fn main() {
    let mut arguments = env::args_os().skip(1);
    let program = arguments.next().expect("the wrapped program is present");
    let selected_target = Path::new(&program)
        .file_stem()
        .is_some_and(|name| name == "optic-rustc-driver");

    let status = if selected_target {
        let rustc = arguments
            .next()
            .expect("the selected rustc path is present");
        Command::new(program)
            .args([rustc, "-vV".into()])
            .status()
            .expect("the compiler probe can run")
    } else {
        Command::new(program)
            .args(arguments)
            .status()
            .expect("the wrapped program can run")
    };

    exit(status.code().unwrap_or(1));
}
