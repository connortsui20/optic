//! Discovers the Cargo workspace that owns a capture request.
//!
//! [`discover_workspace`] asks the Cargo selected by the environment for the canonical workspace
//! root. The resulting [`Workspace`] retains that executable, root, and the absolute invocation
//! directory. Each capture reads metadata again so ordinary manifest edits update the build record.
//! Callers must discover a new [`Workspace`] after relocating the workspace on disk.

use std::env;
use std::path::Path;
use std::path::PathBuf;

use cargo_metadata::CargoOpt;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use snafu::ResultExt;

use crate::BuildRequest;
use crate::Error;
use crate::error::InvocationDirectoryNotAbsoluteSnafu;
use crate::error::MetadataSnafu;

/// Cargo's authoritative view of the workspace used for subsequent builds.
///
/// The invocation directory, workspace root, and member paths must keep their locations for the
/// lifetime of this value.
pub struct Workspace {
    root: PathBuf,
    cargo: PathBuf,
    invocation_directory: PathBuf,
}

impl Workspace {
    /// Returns the canonical workspace root reported by Cargo.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn cargo(&self) -> &Path {
        &self.cargo
    }

    pub(crate) fn invocation_directory(&self) -> &Path {
        &self.invocation_directory
    }

    pub(crate) fn read_metadata(&self, request: &BuildRequest) -> Result<Metadata, Error> {
        query_metadata(&self.cargo, &self.invocation_directory, Some(request))
    }
}

/// Discovers the Cargo workspace containing the absolute `start` path.
///
/// # Errors
///
/// Returns an error if `start` is relative, Cargo cannot be selected, or metadata cannot be read.
pub fn discover_workspace(start: &Path) -> Result<Workspace, Error> {
    if !start.is_absolute() {
        return InvocationDirectoryNotAbsoluteSnafu {
            path: start.to_owned(),
        }
        .fail();
    }

    let cargo = env::var_os("CARGO")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
    let metadata = query_metadata(&cargo, start, None)?;
    let root = metadata.workspace_root.clone().into_std_path_buf();

    Ok(Workspace {
        root,
        cargo,
        invocation_directory: start.to_owned(),
    })
}

fn query_metadata(
    cargo: &Path,
    directory: &Path,
    request: Option<&BuildRequest>,
) -> Result<Metadata, Error> {
    let mut command = MetadataCommand::new();
    command.cargo_path(cargo).current_dir(directory).no_deps();

    if let Some(request) = request {
        if !request.features().is_empty() {
            command.features(CargoOpt::SomeFeatures(request.features().to_vec()));
        }
        if request.all_features() {
            command.features(CargoOpt::AllFeatures);
        }
        if request.no_default_features() {
            command.features(CargoOpt::NoDefaultFeatures);
        }
    }

    command.exec().context(MetadataSnafu)
}
