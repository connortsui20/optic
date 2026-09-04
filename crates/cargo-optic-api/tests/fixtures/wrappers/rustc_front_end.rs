use std::env;
use std::process::Command;
use std::process::exit;

fn main() {
    let status = Command::new("rustc")
        .args(env::args_os().skip(1))
        .status()
        .expect("rustc can run");

    exit(status.code().unwrap_or(1));
}
