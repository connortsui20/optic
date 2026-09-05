use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));

    fs::write(
        output_dir.join("generated.rs"),
        concat!(
            "pub const GENERATED_BY_BUILD_SCRIPT: u64 = 0x1234;\n",
            "pub const GENERATED_ENV: &str = env!(\"OPTIC_RESEARCH_GENERATED_ENV\");\n",
        ),
    )
    .expect("the fixture must write its generated source");

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(optic_research_build_script)");
    println!("cargo::rustc-cfg=optic_research_build_script");
    println!("cargo::rustc-env=OPTIC_RESEARCH_GENERATED_ENV=from-build-script");
}
