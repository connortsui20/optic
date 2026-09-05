//! Collects concrete function instances during one rustc invocation.
//!
//! Cargo starts this executable as a compiler wrapper. It forwards ordinary compiler invocations
//! and enters rustc's internal driver only when it finds the selected target marker.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

mod protocol;

use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;

use protocol::*;
use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_hir::attrs::Linkage;
use rustc_interface::interface::Compiler;
use rustc_middle::mono::MonoItem;
use rustc_middle::mono::Visibility;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::print::with_resolve_crate_name;

struct InstanceCallbacks {
    manifest: ManifestWriter,
    analysis: Option<io::Result<()>>,
}

struct DriverInvocation {
    arguments: Vec<String>,
    manifest_path: PathBuf,
}

struct ConcreteInstance {
    definition_crate: String,
    definition_path: String,
    display_name: String,
    raw_symbol: String,
}

struct Placement {
    codegen_unit: String,
    linkage: String,
    visibility: String,
    local_copy: bool,
    size_estimate: usize,
}

impl Callbacks for InstanceCallbacks {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let partitions = tcx.collect_and_partition_mono_items(());

        for codegen_unit in partitions.codegen_units {
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
                    linkage: linkage_name(data.linkage).to_owned(),
                    visibility: visibility_name(data.visibility).to_owned(),
                    local_copy: data.inlined,
                    size_estimate: data.size_estimate,
                };

                if let Err(error) = self.manifest.write_placement(&concrete, &placement) {
                    self.analysis = Some(Err(error));

                    return Compilation::Stop;
                }
            }
        }

        self.analysis = Some(Ok(()));

        Compilation::Continue
    }
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

fn main() -> ExitCode {
    if env::var_os(DRIVER_INNER_ENV).is_some() {
        run_driver()
    } else {
        run_wrapper()
    }
}

fn run_driver() -> ExitCode {
    let invocation = match prepare_driver_invocation() {
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
    let mut callbacks = InstanceCallbacks {
        manifest,
        analysis: None,
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

fn prepare_driver_invocation() -> Result<DriverInvocation, String> {
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
        return Err("optic rustc driver must receive rustc as its first argument, got none".to_owned());
    }

    let manifest_path = env::var_os(MANIFEST_PATH_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{MANIFEST_PATH_ENV} is not set"))?;

    Ok(DriverInvocation {
        arguments,
        manifest_path,
    })
}

fn run_wrapper() -> ExitCode {
    let mut arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() {
        return failure("optic rustc wrapper must receive a compiler path, got none");
    }

    let Some(marker) = env::var_os(SELECTED_TARGET_MARKER_ENV) else {
        return failure(format!("{SELECTED_TARGET_MARKER_ENV} is not set"));
    };
    let selected_target = arguments.iter().position(|argument| argument == &marker);
    if let Some(index) = selected_target {
        arguments.remove(index);
        let executable = match env::current_exe() {
            Ok(executable) => executable,
            Err(error) => return failure(format!("failed to find the rustc driver: {error}")),
        };
        arguments.insert(0, executable.into_os_string());
    }

    let program = arguments.remove(0);
    let mut command = Command::new(program);
    command.args(arguments);
    if selected_target.is_some() {
        command.env(DRIVER_INNER_ENV, "1");
    }

    execute(command)
}

fn execute(mut command: Command) -> ExitCode {
    use std::os::unix::process::CommandExt;

    let error = command.exec();

    failure(format!("failed to start rustc: {error}"))
}

fn failure(message: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", message.as_ref());

    ExitCode::FAILURE
}

struct ManifestWriter {
    path: PathBuf,
    temporary_path: PathBuf,
    file: BufWriter<File>,
}

impl ManifestWriter {
    fn create(path: &Path) -> io::Result<Self> {
        let temporary_path = path.with_extension("tmp");
        let file = BufWriter::new(File::create(&temporary_path)?);
        let mut writer = Self {
            path: path.to_owned(),
            temporary_path,
            file,
        };
        writer.write_bytes(MANIFEST_MAGIC)?;
        writer.write_u32(PROTOCOL_VERSION)?;

        Ok(writer)
    }

    fn write_placement(
        &mut self,
        instance: &ConcreteInstance,
        placement: &Placement,
    ) -> io::Result<()> {
        self.write_u32(PLACEMENT_RECORD)?;
        self.write_string(&instance.definition_crate)?;
        self.write_string(&instance.definition_path)?;
        self.write_string(&instance.display_name)?;
        self.write_string(&instance.raw_symbol)?;
        self.write_string(&placement.codegen_unit)?;
        self.write_string(&placement.linkage)?;
        self.write_string(&placement.visibility)?;
        self.write_u32(u32::from(placement.local_copy))?;
        self.write_u64(u64::try_from(placement.size_estimate).map_err(|_| {
            invalid_data(format!(
                "placement size estimate must fit in u64, got {}",
                placement.size_estimate
            ))
        })?)
    }

    fn finish(mut self) -> io::Result<()> {
        self.write_u32(END_RECORD)?;
        self.file.flush()?;
        drop(self.file);
        fs::rename(self.temporary_path, self.path)
    }

    fn write_string(&mut self, value: &str) -> io::Result<()> {
        let length = u32::try_from(value.len()).map_err(|_| {
            invalid_data(format!("string length must fit in u32, got {}", value.len()))
        })?;
        self.write_u32(length)?;
        self.write_bytes(value.as_bytes())
    }

    fn write_u32(&mut self, value: u32) -> io::Result<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_u64(&mut self, value: u64) -> io::Result<()> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
