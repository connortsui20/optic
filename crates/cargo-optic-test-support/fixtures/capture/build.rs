fn main() {
    println!("cargo::rerun-if-changed=tracked.txt");
    println!("cargo::rerun-if-env-changed=OPTIC_FIXTURE_VALUE");
    let value = std::env::var("OPTIC_FIXTURE_VALUE")
        .unwrap_or_else(|_| std::fs::read_to_string("tracked.txt").unwrap());
    println!("cargo::rustc-env=OPTIC_FIXTURE_VALUE={}", value.trim());
}
