//! Classifies the selected compiler configuration before retaining optimized bitcode.
//!
//! Only the proven LLVM recipe changes temporary-file retention. Effective provenance comes from
//! the compiler session after backend initialization, never from guessed Cargo profile defaults.

use std::io;
use std::path::Path;

use rustc_interface::interface::Compiler;
use rustc_interface::interface::Config;
use rustc_session::config::Lto;
use rustc_session::config::LtoCli;
use rustc_session::config::OptLevel;
use rustc_target::spec::Target;

/// The compiler source revision exercised by the retained-bitcode unit reproducer.
const VERIFIED_COMMIT: &str = "48a229ceaefd4985c50990b14116b6d856af0985";

pub(crate) struct Configuration {
    /// The backend name reported after its initialization.
    pub(crate) backend: String,
    /// The effective target tuple or target-specification identity.
    pub(crate) target: String,
    /// The effective optimization level, encoded as rustc's CLI spelling.
    pub(crate) optimization: &'static str,
    /// The effective LTO code defined by the private protocol.
    pub(crate) lto: u32,
    /// Whether this invocation uses an incremental compilation directory.
    pub(crate) incremental: bool,
    /// Whether this invocation delegates LTO to the linker plugin.
    pub(crate) linker_plugin: bool,
    /// The configured CGU count, independent of the actual regular module count.
    pub(crate) codegen_units: u32,
    /// The private protocol's unsupported-configuration code, with zero for supported collection.
    pub(crate) unsupported: u32,
}

/// Rejects unsupported configurations before changing either temporary-file option.
pub(crate) fn configure(config: &mut Config, directory: &Path) -> io::Result<u32> {
    let options = &config.opts;
    let (target, _) = Target::search(
        &options.target_triple,
        options.sysroot.path(),
        options.unstable_opts.unstable_options,
    )
    .map_err(io::Error::other)?;
    let backend = options
        .unstable_opts
        .codegen_backend
        .as_deref()
        .or(target.default_codegen_backend.as_deref())
        .unwrap_or("llvm");
    let unsupported = if env!("OPTIC_RUSTC_COMMIT") != VERIFIED_COMMIT {
        6
    } else if backend != "llvm" {
        5
    } else if options.incremental.is_some() {
        1
    } else if options.cg.linker_plugin_lto.enabled() {
        4
    } else if target.requires_lto
        || matches!(options.cg.lto, LtoCli::Yes | LtoCli::NoParam | LtoCli::Fat)
    {
        3
    } else if matches!(options.cg.lto, LtoCli::Thin) {
        2
    } else {
        0
    };

    if unsupported == 0 {
        std::fs::create_dir(directory)?;
        config.opts.cg.save_temps = true;
        config.opts.unstable_opts.temps_dir = Some(
            directory
                .to_str()
                .ok_or_else(|| io::Error::other("compiler temporary directory must be UTF-8"))?
                .to_owned(),
        );
    }

    Ok(unsupported)
}

impl Configuration {
    pub(crate) fn observed(compiler: &Compiler, unsupported: u32) -> io::Result<Self> {
        let session = &compiler.sess;
        let lto = match session.lto() {
            Lto::No => 0,
            Lto::ThinLocal => 1,
            Lto::Thin => 2,
            Lto::Fat => 3,
        };
        let backend = compiler.codegen_backend.name().to_owned();
        if unsupported == 0 && (backend != "llvm" || lto > 1) {
            return Err(io::Error::other(
                "retained LLVM requires the supported backend and LTO mode",
            ));
        }
        let optimization = match session.opts.optimize {
            OptLevel::No => "0",
            OptLevel::Less => "1",
            OptLevel::More => "2",
            OptLevel::Aggressive => "3",
            OptLevel::Size => "s",
            OptLevel::SizeMin => "z",
        };

        Ok(Self {
            backend,
            target: session.opts.target_triple.tuple().to_owned(),
            optimization,
            lto,
            incremental: session.opts.incremental.is_some(),
            linker_plugin: session.opts.cg.linker_plugin_lto.enabled(),
            codegen_units: u32::try_from(session.codegen_units().as_usize())
                .map_err(io::Error::other)?,
            unsupported,
        })
    }

    /// Selects the proven post-optimization write point, not a pre-LTO or no-opt snapshot.
    ///
    /// The `stage-proof.rs` reproducer validates these suffixes against rustc's output naming API.
    /// [No LTO] writes `bc` after optimization. [Local ThinLTO] saves `thin-lto-after-pm` after its
    /// pass manager.
    ///
    /// [No LTO]: https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_codegen_ssa/src/back/write.rs#L819-L840
    /// [Local ThinLTO]: https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_codegen_llvm/src/back/lto.rs#L778-L782
    pub(crate) fn extension(&self) -> Option<&'static str> {
        if self.unsupported != 0 {
            return None;
        }

        Some(if self.lto == 0 {
            "bc"
        } else {
            "thin-lto-after-pm.bc"
        })
    }
}
