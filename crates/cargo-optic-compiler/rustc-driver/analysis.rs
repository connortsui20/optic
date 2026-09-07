//! Drives the selected rustc invocation and extracts its monomorphized functions.
//!
//! Rustc calls [`InstanceCallbacks::after_analysis`] after type analysis. The callback asks rustc
//! for the same mono-item partitions that code generation consumes, then records each function in
//! every codegen unit where rustc placed it. The callback also records every regular codegen unit,
//! including units with no functions, and snapshots supported source from rustc's loaded text.
//! A successful rustc invocation publishes the manifest only if the callback completed without an
//! encoding error. The parent converts expected bitcode only after this compilation succeeds.

use std::env;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_hir::attrs::Linkage;
use rustc_interface::interface::Compiler;
use rustc_interface::interface::Config;
use rustc_middle::mono::CodegenUnit;
use rustc_middle::mono::MonoItem;
use rustc_middle::mono::Visibility;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::print::with_resolve_crate_name;

use crate::failure;
use crate::llvm::Configuration;
use crate::manifest::ConcreteInstance;
use crate::manifest::ManifestWriter;
use crate::manifest::Placement;
use crate::protocol::MANIFEST_PATH_ENV;
use crate::source::Snapshots;

/// Retains one compilation's output until rustc and the analysis callback both succeed.
struct InstanceCallbacks {
    /// Writes the temporary manifest, which [`run`] publishes only after successful compilation.
    manifest: ManifestWriter,

    /// Is absent until [`Callbacks::after_analysis`] runs, then records if collection completed.
    analysis: Option<io::Result<()>>,

    /// Is set by [`Callbacks::config`] and consumed once by [`Self::collect`] before record writes.
    configuration: Option<io::Result<u32>>,

    /// The private attempt directory that owns the manifest, bitcode, and source snapshots.
    directory: PathBuf,

    /// Deduplicates source files across all codegen units in this compilation.
    sources: Snapshots,
}

/// The inner driver's arguments and destinations supplied by the prepared collection attempt.
struct DriverInvocation {
    /// Nonempty UTF-8 arguments, beginning with Cargo's real compiler path.
    arguments: Vec<String>,
    /// An absolute file path inside the private attempt directory, supplied by the collector.
    manifest_path: PathBuf,
    /// The canonical package root supplied by the collector to bound source capture.
    package_root: PathBuf,
}

impl Callbacks for InstanceCallbacks {
    fn config(&mut self, config: &mut Config) {
        self.configuration = Some(crate::llvm::configure(config, &self.directory.join("llvm")));
    }

    fn after_analysis<'tcx>(&mut self, compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let result = self.collect(compiler, tcx);
        let compilation = if result.is_ok() {
            Compilation::Continue
        } else {
            Compilation::Stop
        };

        self.analysis = Some(result);

        compilation
    }
}

impl InstanceCallbacks {
    /// Writes configuration before module and placement records in rustc's codegen-unit order.
    fn collect<'tcx>(&mut self, compiler: &Compiler, tcx: TyCtxt<'tcx>) -> io::Result<()> {
        let unsupported = self
            .configuration
            .take()
            .expect("rustc called config before after_analysis to set the configuration result")?;
        let configuration = Configuration::observed(compiler, unsupported)?;
        self.manifest.write_configuration(&configuration)?;

        let partitions = tcx.collect_and_partition_mono_items(());

        for codegen_unit in partitions.codegen_units {
            if let Some(extension) = configuration.extension() {
                let name = codegen_unit.name().to_string();
                let path = tcx
                    .output_filenames(())
                    .temp_path_ext_for_cgu(extension, &name);

                self.manifest.write_module(&name, &path)?;
            }

            self.write_function_placements(tcx, codegen_unit)?;
        }

        Ok(())
    }

    /// Records each function copy in one codegen unit, excluding static data and assembly.
    fn write_function_placements<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        codegen_unit: &CodegenUnit<'tcx>,
    ) -> io::Result<()> {
        for (mono_item, data) in codegen_unit.items_in_deterministic_order(tcx) {
            let MonoItem::Fn(instance) = mono_item else {
                continue;
            };

            let definition_id = instance.def_id();
            let concrete = ConcreteInstance {
                definition_crate: tcx.crate_name(definition_id.krate).to_string(),
                definition_path: with_resolve_crate_name!(with_no_trimmed_paths!(
                    tcx.def_path_str(definition_id)
                )),
                display_name: with_resolve_crate_name!(with_no_trimmed_paths!(
                    tcx.def_path_str_with_args(definition_id, instance.args)
                )),
                raw_symbol: tcx.symbol_name(instance).name.to_owned(),
            };
            let placement = Placement {
                codegen_unit: codegen_unit.name().to_string(),
                linkage: linkage_name(data.linkage),
                visibility: visibility_name(data.visibility),
                local_copy: data.inlined,
                size_estimate: data.size_estimate,
            };

            let source = self.sources.capture(tcx, instance, &mut self.manifest)?;
            self.manifest
                .write_placement(&concrete, &placement, &source)?;
        }

        Ok(())
    }
}

/// Runs rustc with the callback that writes the selected target's manifest.
pub(crate) fn run() -> ExitCode {
    let invocation = match prepare_invocation() {
        Ok(invocation) => invocation,
        Err(error) => return failure(error),
    };

    let manifest = match ManifestWriter::create(&invocation.manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            return failure(format!(
                "failed to create {}: {error}",
                invocation.manifest_path.display()
            ));
        }
    };

    let directory = invocation
        .manifest_path
        .parent()
        .expect("the collector supplies a manifest file path inside its attempt directory")
        .to_owned();
    let mut callbacks = InstanceCallbacks {
        manifest,
        analysis: None,
        configuration: None,
        sources: Snapshots::new(directory.clone(), invocation.package_root),
        directory,
    };

    let exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&invocation.arguments, &mut callbacks);
    });

    if exit_code != ExitCode::SUCCESS {
        return exit_code;
    }

    match callbacks.analysis {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            return failure(format!(
                "failed to write {}: {error}",
                invocation.manifest_path.display()
            ));
        }
        // Rustc can finish successfully without analysis for informational invocations. The outer
        // collector detects a missing manifest when it expected selected-target analysis.
        None => return ExitCode::SUCCESS,
    }

    match callbacks.manifest.finish() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => failure(format!(
            "failed to finish {}: {error}",
            invocation.manifest_path.display()
        )),
    }
}

/// Reads the rustc argument vector and manifest destination supplied by the outer wrapper.
fn prepare_invocation() -> Result<DriverInvocation, String> {
    let arguments = env::args_os()
        .skip(1)
        .map(|argument| {
            argument.into_string().map_err(|_| {
                "optic rustc driver requires UTF-8 compiler arguments, got a non-UTF-8 argument"
                    .to_owned()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if arguments.is_empty() {
        return Err(
            "optic rustc driver must receive rustc as its first argument, got none".to_owned(),
        );
    }

    let manifest_path = env::var_os(MANIFEST_PATH_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{MANIFEST_PATH_ENV} is not set"))?;
    let package_root = env::var_os(crate::protocol::PACKAGE_ROOT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{} is not set", crate::protocol::PACKAGE_ROOT_ENV))?;

    Ok(DriverInvocation {
        arguments,
        manifest_path,
        package_root,
    })
}

fn linkage_name(linkage: Linkage) -> &'static str {
    match linkage {
        Linkage::AvailableExternally => "AvailableExternally",
        Linkage::Common => "Common",
        Linkage::ExternalWeak => "ExternalWeak",
        Linkage::External => "External",
        Linkage::Internal => "Internal",
        Linkage::LinkOnceAny => "LinkOnceAny",
        Linkage::LinkOnceODR => "LinkOnceODR",
        Linkage::WeakAny => "WeakAny",
        Linkage::WeakODR => "WeakODR",
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Default => "Default",
        Visibility::Hidden => "Hidden",
        Visibility::Protected => "Protected",
    }
}
