//! Controls fixture invalidation through a tracked file and an environment variable.
//!
//! Cargo forwards this value to the fixture so tests can observe build-script input changes.

fn main() {
    println!("cargo::rerun-if-changed=tracked.txt");
    println!("cargo::rerun-if-env-changed=OPTIC_FIXTURE_VALUE");

    let value = std::env::var("OPTIC_FIXTURE_VALUE").unwrap_or_else(|_| {
        std::fs::read_to_string("tracked.txt")
            .expect("the capture fixture includes tracked.txt beside build.rs")
    });

    println!("cargo::rustc-env=OPTIC_FIXTURE_VALUE={}", value.trim());
}
