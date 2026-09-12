//! Exercises real compilation protocols, source snapshots, and optimized artifact stages.
//!
//! Each journey runs in an isolated child through exported compiler APIs or the standalone proof.
//! Private provisioning counters and corrupt-bitcode collection remain in compiler unit tests.

#![cfg(unix)]

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use cargo_optic_test_support::TestWorkspace;
use optic_compiler::BuildRequest;
use optic_compiler::CargoTarget;
use optic_compiler::Freshness;
use optic_compiler::discover_workspace;
use optic_compiler::prepare_build;
use optic_records::CaptureId;
use optic_records::LlvmCollection;
use optic_records::LlvmLto;
use optic_records::LlvmStage;
use optic_records::SourceAvailability;
use optic_records::SourceUnavailable;
use optic_records::UnsupportedLlvmConfiguration;

#[track_caller]
fn child(fixture: &TestWorkspace, test: &str) -> std::process::Output {
    let mut command = Command::new(env::current_exe().unwrap());
    fixture.apply(&mut command);

    command
        .args(["--exact", test, "--nocapture"])
        .env("OPTIC_TEST_CHILD", "1");

    let output = cargo_optic_test_support::run(&mut command);
    cargo_optic_test_support::assert_success(&command, &output);

    output
}

/// Checks retained optimization results and unchanged settings for one LTO configuration.
///
/// The root contains the compiled `stage-proof` driver and its `input.rs` fixture. Each run gets a
/// separate directory so that assertions about retained files cannot observe an earlier invocation.
#[track_caller]
fn assert_retained_stage(root: &Path, llvm_dis: &Path, label: &str, lto: Option<&str>) {
    let driver = root.join("stage-proof");
    let input = root.join("input.rs");
    let mut configurations = Vec::new();

    for retain in [
        false, // Compile without retaining bitcode.
        true,  // Compile with retained bitcode.
    ] {
        let directory = root.join(format!("{label}-{retain}"));
        fs::create_dir(&directory).unwrap();
        let executable = directory.join("proof-input");
        let mut command = Command::new(&driver);
        command
            .arg(&input)
            .args([
                "--crate-name",
                "proof_input",
                "--edition=2024",
                "-C",
                "opt-level=3",
                "-C",
                "codegen-units=4",
            ])
            .arg("-o")
            .arg(&executable)
            .env("OPTIC_PROOF_DIRECTORY", &directory);

        if let Some(lto) = lto {
            command.arg("-C").arg(lto);
        }

        if retain {
            command.env("OPTIC_PROOF_RETAIN", "1");
        }

        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{command:?}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(Command::new(&executable).status().unwrap().success());
        configurations.push(fs::read_to_string(directory.join("configuration")).unwrap());

        if !retain {
            continue;
        }

        let modules = fs::read_to_string(directory.join("modules")).unwrap();
        assert!(modules.lines().count() > 1, "{modules}");
        let mut folded = 0;

        for module in modules.lines() {
            folded += assert_module_folding(llvm_dis, module);
        }

        assert_eq!(folded, 2);
        eprintln!(
            "{label}: {} regular modules\n{}",
            modules.lines().count(),
            configurations.last().unwrap()
        );
    }

    assert_eq!(configurations[0], configurations[1]);
}

/// Counts fixture definitions folded only in this module's selected optimized output.
#[track_caller]
fn assert_module_folding(llvm_dis: &Path, module: &str) -> usize {
    let (_, paths) = module.split_once('\t').unwrap();
    let (path, before) = paths.split_once('\t').unwrap();
    assert!(fs::metadata(path).unwrap().len() > 0, "{path}");

    let text = disassemble(llvm_dis, path);
    let mut folded = 0;

    for (symbol, expected) in [
        ("folded_first", "ret i64 42"),  // First fixture definition.
        ("folded_second", "ret i64 17"), // Second fixture definition.
    ] {
        let Some(body) = function_body(&text, symbol) else {
            continue;
        };

        assert!(body.contains(expected), "{path}: {body}");

        let before = disassemble(llvm_dis, before);
        let body = function_body(&before, symbol).unwrap();
        assert!(
            !body.contains(expected),
            "the no-opt module must precede constant folding: {body}"
        );
        folded += 1;
    }

    folded
}

#[track_caller]
fn disassemble(llvm_dis: &Path, bitcode: &str) -> String {
    let output = Command::new(llvm_dis)
        .args([bitcode, "-o", "-"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{bitcode}: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
}

/// Finds an unquoted fixture definition in llvm-dis output.
#[track_caller]
fn function_body<'a>(text: &'a str, symbol: &str) -> Option<&'a str> {
    let header = text
        .lines()
        .find(|line| line.starts_with("define ") && line.contains(&format!("@{symbol}(")))?;
    let start = text.find(header).unwrap();

    Some(text[start..].split_once("\n}").unwrap().0)
}

#[test]
fn driver_protocol_stops_stale_probes_and_collects_new_tokens() {
    let output = child(&TestWorkspace::new("capture"), "driver_protocol_child");

    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("stopped a stale selected-target probe")
    );
}

#[test]
fn failed_retry_replays_probe_and_collection_diagnostics() {
    let output = child(&TestWorkspace::new("capture"), "failed_retry_child");
    let diagnostics = String::from_utf8_lossy(&output.stderr);

    assert!(
        diagnostics.contains("stopped a stale selected-target probe"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("expected"), "{diagnostics}");
}

#[test]
fn failed_retry_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "release").unwrap();
    let (_, _, analysis, _, _temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    fs::write(
        workspace.root().join("src/lib.rs"),
        "pub fn broken() { let = ; }\n",
    )
    .unwrap();

    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&analysis).unwrap(), Freshness::Stale);
    assert!(prepared.collect().is_err());
}

#[test]
fn driver_protocol_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "checked").unwrap();
    let (_, _, analysis, manifest, _temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();

    assert!(
        manifest
            .instances()
            .iter()
            .any(|instance| instance.display_name().contains("captured_value"))
    );
    assert_eq!(analysis.artifact().artifact().profile.opt_level, "1");
    assert!(analysis.artifact().artifact().profile.debug_assertions);
    assert_eq!(manifest.llvm_provenance().optimization(), "1");
    assert_eq!(manifest.llvm_provenance().lto(), LlvmLto::LocalThin);
    let LlvmCollection::Collected(modules) = manifest.llvm() else {
        panic!("the nonincremental LLVM target has a proven stage");
    };

    assert!(!modules.is_empty());
    assert!(
        modules
            .iter()
            .all(|module| module.stage() == LlvmStage::LocalThinLtoPostPassManager)
    );

    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&analysis).unwrap(), Freshness::Fresh);

    fs::write(
        workspace.root().join("src/lib.rs"),
        "pub fn kernel(value: u64) -> u64 { value + 200 }\n",
    )
    .unwrap();

    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&analysis).unwrap(), Freshness::Stale);

    let (_, _, changed, _, _changed_temporary) = prepared
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    assert_ne!(changed.token(), analysis.token());
    assert_eq!(changed.request_key(), analysis.request_key());

    let mut prepared = prepare_build(&workspace, &request).unwrap();
    assert_eq!(prepared.probe(&changed).unwrap(), Freshness::Fresh);

    let (_, _, forced, _, _forced_temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    assert_ne!(forced.token(), changed.token());
}

#[test]
fn retained_bitcode_proves_no_lto_and_local_thin_lto_stages() {
    let output = child(
        &TestWorkspace::new("capture"),
        "retained_bitcode_stage_child",
    );
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn retained_bitcode_stage_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let compiler = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    assert!(compiler.status.success());

    let sysroot = PathBuf::from(String::from_utf8(compiler.stdout).unwrap().trim());
    let rustc = sysroot.join("bin/rustc");
    let version = Command::new(&rustc).arg("-vV").output().unwrap();
    let version = String::from_utf8(version.stdout).unwrap();
    assert!(version.contains("commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985"));
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    let llvm_dis = sysroot.join("lib/rustlib").join(host).join("bin/llvm-dis");

    let driver = root.join("stage-proof");
    let source = root.join("stage-proof.rs");
    fs::write(&source, include_str!("fixtures/stage-proof.rs")).unwrap();
    let mut compile = Command::new(&rustc);
    compile
        .args([
            "--crate-name",
            "optic_stage_proof",
            "--edition=2024",
            "-C",
            "prefer-dynamic",
            "-C",
            "rpath",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&driver)
        .env("RUSTC_BOOTSTRAP", "optic_stage_proof");

    let output = compile.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let input = root.join("input.rs");
    fs::write(
        &input,
        concat!(
            "\npub mod first {\n",
            "    #[unsafe(no_mangle)]\n",
            "    #[inline(never)]\n",
            "    pub fn folded_first(value: u64) -> u64 { \
                 value.wrapping_add(7).wrapping_sub(value).wrapping_add(35) }\n",
            "}\n\npub mod second {\n",
            "    #[unsafe(no_mangle)]\n",
            "    #[inline(never)]\n",
            "    pub fn folded_second(value: u64) -> u64 { \
                 value.wrapping_add(7).wrapping_sub(value).wrapping_add(10) }\n",
            "}\n",
            "fn main() { assert_eq!(first::folded_first(10) + second::folded_second(20), 59); }\n",
        ),
    )
    .unwrap();

    for (label, lto) in [
        ("no-lto", Some("lto=off")), // Explicitly disable all LTO.
        ("local-thin", None),        // Retain the compiler's default local ThinLTO.
    ] {
        assert_retained_stage(root, &llvm_dis, label, lto);
    }
}

#[test]
fn placements_preserve_compiler_metadata_and_names() {
    child(&TestWorkspace::new("capture"), "placement_metadata_child");
}

#[test]
fn placement_metadata_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    fs::write(
        workspace.root().join("src/lib.rs"),
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn exported(value: u64) -> u64 { helper(value) }\n\
         #[inline(always)]\n\
         fn helper(value: u64) -> u64 { value.wrapping_add(1) }\n",
    )
    .unwrap();

    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "dev").unwrap();
    let (_, _, _, manifest, _temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();

    let [exported, helper] = ["capture_fixture::exported", "capture_fixture::helper"].map(|name| {
        let instances: Vec<_> = manifest
            .instances()
            .iter()
            .filter(|instance| instance.definition().definition_path() == name)
            .collect();
        assert_eq!(instances.len(), 1, "{name}");
        let instance = instances[0];
        assert_eq!(instance.definition().crate_name(), "capture_fixture");
        assert_eq!(instance.display_name(), name);
        assert_eq!(instance.placements().len(), 1, "{name}");

        instance
    });
    assert_eq!(exported.raw_symbol(), "exported");
    assert_ne!(helper.raw_symbol(), exported.raw_symbol());

    for (instance, linkage, local_copy) in [
        (exported, "External", false), // The exported function owns its global definition.
        (helper, "Internal", true),    // The caller's codegen unit owns a local helper copy.
    ] {
        let placement = &instance.placements()[0];
        assert_eq!(placement.linkage(), linkage);
        assert_eq!(placement.visibility(), "Default");
        assert_eq!(placement.local_copy(), local_copy);
        assert!(placement.size_estimate() > 0);
        assert_eq!(
            placement.codegen_unit(),
            exported.placements()[0].codegen_unit()
        );
    }
}

#[test]
fn source_lines_follow_each_normalized_file_for_repeated_instances() {
    child(&TestWorkspace::new("capture"), "source_lines_child");
}

#[test]
fn source_lines_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let first = "pub fn first<T: Copy>(value: T) -> T { value }";
    let later = "pub fn later() -> u64 { first(1_u64) + u64::from(first(2_u32)) + other::first() }";
    let other_first = "pub fn first() -> u64 { later(3_u64) + u64::from(later(4_u32)) }";
    let other_later = "pub fn later<T: Copy>(value: T) -> T { value }";
    let root =
        format!("{first}\n// Unicode before a later definition: λ.\n\n{later}\nmod other;\n");
    let other = format!("{other_first}\n\n// Another Unicode prefix: é.\n\n{other_later}\n");

    for (file, normalized) in [
        ("lib.rs", &root),    // Use the root module.
        ("other.rs", &other), // Use a separate source module.
    ] {
        fs::write(
            workspace.root().join("src").join(file),
            format!("\u{feff}{}", normalized.replace('\n', "\r\n")),
        )
        .unwrap();
    }

    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "dev").unwrap();
    let (_, _, _, manifest, temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    assert_eq!(manifest.artifacts().len(), 2);

    for (name, file, normalized, definition, line, count) in [
        ("capture_fixture::first::<", "lib.rs", &root, first, 1, 2), // First-line instances.
        ("capture_fixture::later", "lib.rs", &root, later, 4, 1),    // Later root definition.
        (
            "capture_fixture::other::first",
            "other.rs",
            &other,
            other_first,
            1,
            1,
        ), // New file.
        (
            "capture_fixture::other::later::<",
            "other.rs",
            &other,
            other_later,
            5,
            2,
        ), // Repeated instances.
    ] {
        let instances: Vec<_> = manifest
            .instances()
            .iter()
            .filter(|instance| instance.display_name().starts_with(name))
            .collect();
        assert_eq!(instances.len(), count, "{name}");

        for instance in &instances {
            assert_eq!(instance.source(), instances[0].source(), "{name}");
            let SourceAvailability::Available(source) = instance.source() else {
                panic!("the local definition has source: {name}");
            };

            let artifact = manifest
                .artifacts()
                .iter()
                .find(|artifact| artifact.id() == source.artifact())
                .unwrap();
            let snapshot = fs::read_to_string(temporary.path().join(artifact.file_name())).unwrap();
            assert_eq!(&snapshot, normalized);

            let start = normalized.find(definition).unwrap();
            assert_eq!(source.range().start(), start as u64);
            assert_eq!(source.range().end(), (start + definition.len()) as u64);
            assert_eq!(source.starting_line(), line);
            assert_eq!(
                source.display_path(),
                fs::canonicalize(workspace.root().join("src").join(file)).unwrap()
            );
        }
    }
}

#[test]
fn source_snapshots_distinguish_compiler_files_with_one_canonical_path() {
    child(&TestWorkspace::new("capture"), "source_aliases_child");
}

#[test]
fn source_aliases_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let path = workspace.root().join("src/shared.rs");
    let definition = "pub fn shared<T: Copy>(value: T) -> T { value }";
    let text = format!("#[inline(never)]\n{definition}\n");
    fs::write(&path, &text).unwrap();
    fs::create_dir(workspace.root().join("src/inner")).unwrap();
    fs::write(
        workspace.root().join("src/lib.rs"),
        r#"#[path = "shared.rs"]
mod direct;
#[path = "inner/../shared.rs"]
mod alias;
pub fn uses_both(value: u64, other: u32) -> u64 {
    direct::shared(value) + u64::from(direct::shared(other))
        + alias::shared(value) + u64::from(alias::shared(other))
}
"#,
    )
    .unwrap();

    let mut command = Command::new(env::var_os("CARGO").unwrap());
    command.args(["rustc", "--lib", "--release"]);
    let output = cargo_optic_test_support::run(&mut command);
    cargo_optic_test_support::assert_success(&command, &output);

    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "release").unwrap();
    let (_, _, _, manifest, temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    let sources = ["direct", "alias"].map(|module| {
        let prefix = format!("capture_fixture::{module}::shared::<");
        let instances: Vec<_> = manifest
            .instances()
            .iter()
            .filter(|instance| instance.display_name().starts_with(&prefix))
            .collect();
        assert_eq!(instances.len(), 2, "{module}");
        assert_eq!(instances[0].source(), instances[1].source());
        let SourceAvailability::Available(source) = instances[0].source() else {
            panic!("the local shared definition has source: {module}");
        };

        let artifact = manifest
            .artifacts()
            .iter()
            .find(|artifact| artifact.id() == source.artifact())
            .unwrap();
        let snapshot = fs::read_to_string(temporary.path().join(artifact.file_name())).unwrap();
        assert_eq!(snapshot, text);
        let start = usize::try_from(source.range().start()).unwrap();
        let end = usize::try_from(source.range().end()).unwrap();
        assert_eq!(&snapshot[start..end], definition);
        assert_eq!(source.starting_line(), 2);
        assert_eq!(source.display_path(), fs::canonicalize(&path).unwrap());

        source
    });
    assert_ne!(sources[0].artifact(), sources[1].artifact());
}

#[test]
fn source_snapshots_preserve_normalized_whole_definitions_for_unsupported_llvm() {
    child(&TestWorkspace::new("capture"), "source_snapshots_child");
}

#[test]
fn source_snapshots_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let path = workspace.root().join("src/lib.rs");
    let normalized = r#"// A Unicode prefix: λ.
#[inline(never)]
pub fn generic<T: Copy>(value: T) -> T {
    value
}
pub fn uses_both() -> u64 {
    generic(10_u64) + u64::from(generic(20_u32)) + generated() + outside::outside()
        + fixture_dependency::identity(3_u64)
}
macro_rules! generate { () => { pub fn generated() -> u64 { 4 } }; }
generate!();
#[path = "outside.rs"]
mod outside;
"#;
    fs::write(
        &path,
        format!("\u{feff}{}", normalized.replace('\n', "\r\n")),
    )
    .unwrap();
    let outside = PathBuf::from(env::var_os("TMPDIR").unwrap()).join("outside.rs");
    fs::write(&outside, "pub fn outside() -> u64 { 5 }\n").unwrap();
    std::os::unix::fs::symlink(&outside, workspace.root().join("src/outside.rs")).unwrap();
    fs::write(
        workspace.root().join("dependency/src/lib.rs"),
        "pub fn identity<T>(value: T) -> T { value }\n",
    )
    .unwrap();

    let request = BuildRequest::new("capture_fixture", CargoTarget::Library, "dev").unwrap();
    let (_, _, _, manifest, temporary) = prepare_build(&workspace, &request)
        .unwrap()
        .collect()
        .unwrap()
        .into_parts(CaptureId::generate())
        .unwrap();
    assert_eq!(
        manifest.llvm(),
        &LlvmCollection::NotCaptured(UnsupportedLlvmConfiguration::Incremental)
    );
    assert!(manifest.llvm_provenance().incremental());
    assert!(!temporary.path().join("llvm").exists());
    assert_eq!(manifest.artifacts().len(), 1);
    assert_eq!(
        fs::read_to_string(temporary.path().join(manifest.artifacts()[0].file_name())).unwrap(),
        normalized
    );

    let generic: Vec<_> = manifest
        .instances()
        .iter()
        .filter(|instance| instance.display_name().contains("::generic::<"))
        .collect();
    assert_eq!(generic.len(), 2);
    assert_eq!(generic[0].source(), generic[1].source());
    let SourceAvailability::Available(source) = generic[0].source() else {
        panic!("the local generic definition has source")
    };

    let start = usize::try_from(source.range().start()).unwrap();
    let end = usize::try_from(source.range().end()).unwrap();
    assert_eq!(
        &normalized[start..end],
        "pub fn generic<T: Copy>(value: T) -> T {\n    value\n}"
    );
    assert_eq!(source.starting_line(), 3);
    assert_eq!(source.display_path(), fs::canonicalize(&path).unwrap());

    let generated = manifest
        .instances()
        .iter()
        .find(|instance| instance.display_name().ends_with("::generated"))
        .unwrap();
    assert_eq!(
        generated.source(),
        &SourceAvailability::Unavailable(SourceUnavailable::Generated)
    );

    let outside = manifest
        .instances()
        .iter()
        .find(|instance| instance.display_name().ends_with("::outside::outside"))
        .unwrap();
    assert_eq!(
        outside.source(),
        &SourceAvailability::Unavailable(SourceUnavailable::OutsidePackage)
    );

    let dependency = manifest
        .instances()
        .iter()
        .find(|instance| {
            instance
                .display_name()
                .contains("fixture_dependency::identity")
        })
        .unwrap();
    assert_eq!(
        dependency.source(),
        &SourceAvailability::Unavailable(SourceUnavailable::Nonlocal)
    );

    fs::write(&path, "// Changed after collection.\n").unwrap();
    assert_eq!(
        fs::read_to_string(temporary.path().join(manifest.artifacts()[0].file_name())).unwrap(),
        normalized
    );
}

#[test]
fn explicit_lto_modes_preserve_source_and_effective_configuration() {
    child(&TestWorkspace::new("capture"), "explicit_lto_modes_child");
}

#[test]
fn explicit_lto_modes_child() {
    if env::var_os("OPTIC_TEST_CHILD").is_none() {
        return;
    }

    let workspace = discover_workspace(&env::current_dir().unwrap()).unwrap();
    let path = workspace.root().join("Cargo.toml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(
        workspace.root().join("src/lib.rs"),
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn exported(value: u64) -> u64 { value + 1 }\n",
    )
    .unwrap();
    fs::write(
        workspace.root().join("src/generic.rs"),
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn exported(value: u64) -> u64 { value + 1 }\n\
         fn main() { println!(\"{}\", exported(41)); }\n",
    )
    .unwrap();
    fs::write(
        &path,
        format!(
            "{original}\n\
             [profile.off]\n\
             inherits = 'release'\n\
             lto = 'off'\n\
             codegen-units = 4\n\
             [profile.thin]\n\
             inherits = 'release'\n\
             lto = 'thin'\n\
             [profile.fat]\n\
             inherits = 'release'\n\
             lto = 'fat'\n"
        ),
    )
    .unwrap();

    for (profile, expected_lto, unsupported) in [
        ("off", LlvmLto::Off, None), // Explicitly disabled local and cross-crate LTO.
        (
            "thin",
            LlvmLto::CrossCrateThin,
            Some(UnsupportedLlvmConfiguration::CrossCrateThinLto),
        ), // Cross-crate ThinLTO.
        (
            "fat",
            LlvmLto::Fat,
            Some(UnsupportedLlvmConfiguration::FatLto),
        ), // Fat LTO.
    ] {
        let request = BuildRequest::new(
            "capture_fixture",
            CargoTarget::Binary("generic".to_owned()),
            profile,
        )
        .unwrap();
        let (_, _, _, manifest, temporary) = prepare_build(&workspace, &request)
            .unwrap()
            .collect()
            .unwrap()
            .into_parts(CaptureId::generate())
            .unwrap();
        assert_eq!(manifest.llvm_provenance().lto(), expected_lto);
        assert_eq!(manifest.llvm_provenance().optimization(), "3");
        assert!(
            manifest
                .instances()
                .iter()
                .any(|instance| matches!(instance.source(), SourceAvailability::Available(_))),
            "{manifest:#?}"
        );

        if let Some(reason) = unsupported {
            assert_eq!(manifest.llvm(), &LlvmCollection::NotCaptured(reason));
            assert!(!temporary.path().join("llvm").exists());

            continue;
        }

        assert_eq!(manifest.llvm_provenance().codegen_units(), 4);
        let LlvmCollection::Collected(modules) = manifest.llvm() else {
            panic!("no-LTO optimized LLVM is supported")
        };

        assert!(!modules.is_empty());
        assert!(
            modules
                .iter()
                .all(|module| module.stage() == LlvmStage::NoLtoOptimized)
        );
    }
}
