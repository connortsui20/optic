//! Discovers the Cargo workspace that owns a capture request.
//!
//! [`discover_workspace`] asks the Cargo selected by the environment for metadata and retains that
//! executable with the resulting [`Workspace`]. Later builds reuse both values, preventing
//! discovery and execution from silently referring to different Cargo installations or workspaces.

use std::env;
use std::path::Path;
use std::path::PathBuf;

use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use snafu::ResultExt;

use crate::Error;
use crate::MetadataSnafu;

/// Cargo's authoritative view of the workspace used for subsequent builds.
pub struct Workspace {
    root: PathBuf,
    cargo: PathBuf,
    metadata: Metadata,
}

impl Workspace {
    /// Returns the canonical workspace root reported by Cargo.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn cargo(&self) -> &Path {
        &self.cargo
    }

    pub(crate) fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

/// Discovers the Cargo workspace containing `start`.
///
/// # Errors
///
/// Returns an error if Cargo cannot be selected or metadata cannot be read.
pub fn discover_workspace(start: &Path) -> Result<Workspace, Error> {
    let cargo = env::var_os("CARGO")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
    let mut command = MetadataCommand::new();
    command.cargo_path(&cargo).current_dir(start).no_deps();

    let metadata = command.exec().context(MetadataSnafu)?;
    let root = metadata.workspace_root.clone().into_std_path_buf();

    Ok(Workspace {
        root,
        cargo,
        metadata,
    })
}
