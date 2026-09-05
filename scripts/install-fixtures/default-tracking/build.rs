//! Keeps Cargo's default package-file tracking active for the installation journey.
//!
//! No rerun instructions exclude Optic output before the first capture initializes the store.

fn main() {
    println!("cargo::rustc-cfg=default_tracking_fixture");
}
