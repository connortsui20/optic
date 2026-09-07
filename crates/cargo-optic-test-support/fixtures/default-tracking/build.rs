//! Leaves Cargo's default package-file tracking active for capture tests.
//!
//! Omitting rerun instructions lets tests check whether Optic output changes the package inputs.

fn main() {
    println!("cargo::rustc-cfg=default_tracking_fixture");
}
