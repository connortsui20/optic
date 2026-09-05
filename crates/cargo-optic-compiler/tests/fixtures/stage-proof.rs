//! Proves the retained LLVM stage without changing the selected code-generation configuration.
//!
//! The compiler integration test builds this standalone callback with the pinned rustc. Paired
//! invocations differ only in temporary-file retention and its directory. Both record the effective
//! session configuration and the complete regular codegen-unit list before code generation.
//!
//! Rust 1.98.1 writes the no-LTO `bc` file in [`codegen`] after optimization. Local ThinLTO saves
//! `thin-lto-after-pm.bc` immediately after [`run_pass_manager`]. Both paths use [`OutputFilenames`],
//! so module identity and the configured output stem determine the expected file without a glob.
//!
//! [codegen]: https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_codegen_llvm/src/back/write.rs#L968-L1028
//! [run_pass_manager]: https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_codegen_llvm/src/back/lto.rs#L778-L782
//! [OutputFilenames]: https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_session/src/config.rs#L1266-L1285

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;

use std::env;
use std::fs;
use std::path::PathBuf;

use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_interface::interface::Compiler;
use rustc_interface::interface::Config;
use rustc_middle::ty::TyCtxt;
use rustc_session::config::Lto;

struct Proof {
    directory: PathBuf,
    retain: bool,
}

impl Callbacks for Proof {
    fn config(&mut self, config: &mut Config) {
        if self.retain {
            config.opts.cg.save_temps = true;
            config.opts.unstable_opts.temps_dir = Some(self.directory.display().to_string());
        }
    }

    fn after_analysis<'tcx>(&mut self, _: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let session = tcx.sess;
        let lto = match session.lto() {
            Lto::No => "no",
            Lto::ThinLocal => "thin-local",
            Lto::Thin => "thin",
            Lto::Fat => "fat",
        };
        let configuration = format!(
            "opt={:?}\nlto={:?}\ncgus={:?}\nemits={:?}\n",
            session.opts.optimize,
            lto,
            session.codegen_units().as_usize(),
            session.opts.output_types,
        );
        fs::write(self.directory.join("configuration"), configuration).unwrap();

        let extension = match session.lto() {
            Lto::No => "bc",
            Lto::ThinLocal => "thin-lto-after-pm.bc",
            _ => panic!("the proof only supports no LTO and local ThinLTO, got {lto}"),
        };
        let outputs = tcx.output_filenames(());
        let mut modules = String::new();

        for cgu in tcx.collect_and_partition_mono_items(()).codegen_units {
            let name = cgu.name().to_string();
            let path = outputs.temp_path_ext_for_cgu(extension, &name);
            let before = outputs.temp_path_ext_for_cgu("no-opt.bc", &name);
            modules.push_str(&format!(
                "{name}\t{}\t{}\n",
                path.display(),
                before.display()
            ));
        }

        fs::write(self.directory.join("modules"), modules).unwrap();

        Compilation::Continue
    }
}

fn main() {
    let directory = PathBuf::from(env::var_os("OPTIC_PROOF_DIRECTORY").unwrap());
    let retain = env::var_os("OPTIC_PROOF_RETAIN").is_some();
    let arguments: Vec<String> = env::args().collect();

    rustc_driver::run_compiler(&arguments, &mut Proof { directory, retain });
}
