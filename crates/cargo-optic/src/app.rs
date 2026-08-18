//! Coordinates compiler capture, persistence, lookup, and inspection.
//!
//! [`Application`] is the single product entry point. It serializes capture mutations while keeping
//! every completed query read-only and independent of shared client state.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_ir::{BuildRequest, CaptureOutcome};

use crate::source::{SourceBaseline, find_item};
use crate::store::Store;
use crate::{
    BuildSpec, CachePolicy, CaptureId, CaptureSummary, FindResult, InstanceId, Result, ShowView,
};

/// Product workflows for one Cargo workspace and its `.optic` store.
pub struct Application {
    workspace_root: PathBuf,
    target_directory: PathBuf,
    store: Store,
}

impl Application {
    /// Opens the project store for the Cargo workspace containing `directory`.
    pub fn discover(directory: &Path, manifest_path: Option<&Path>) -> Result<Self> {
        let metadata = metadata(directory, manifest_path)?;
        let workspace_root = metadata.workspace_root.into_std_path_buf();
        let target_directory = metadata.target_directory.into_std_path_buf();
        let store = Store::open(&workspace_root)?;

        Ok(Self {
            workspace_root,
            target_directory,
            store,
        })
    }

    /// Captures or reuses enriched compiler evidence for one Cargo target.
    pub fn capture(
        &mut self,
        spec: &BuildSpec,
        cache_policy: CachePolicy,
    ) -> Result<CaptureSummary> {
        let _writer = self.store.lock_writer()?;
        let toolchain = cargo_ir::inspect_toolchain()?;
        let capture_id = CaptureId::new();
        let staging = self.store.staging_directory(&capture_id);
        fs::create_dir_all(&staging)
            .map_err(|source| crate::Error::filesystem("create", &staging, source))?;

        let result = self.capture_to_staging(spec, cache_policy, &toolchain, &capture_id, &staging);
        let cleanup = remove_staging(&staging);

        match (result, cleanup) {
            (Ok(summary), Ok(())) => Ok(summary),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn capture_to_staging(
        &mut self,
        spec: &BuildSpec,
        cache_policy: CachePolicy,
        toolchain: &cargo_ir::Toolchain,
        capture_id: &CaptureId,
        staging: &Path,
    ) -> Result<CaptureSummary> {
        let sources = SourceBaseline::capture(&self.workspace_root, spec, staging)?;
        let request_key = request_key(
            spec,
            toolchain,
            &self.target_directory,
            sources.cache_digest(),
        )?;

        if cache_policy == CachePolicy::Reuse
            && let Some(summary) = self.store.cached_capture(&request_key)?
        {
            return Ok(summary);
        }

        let request = self.build_request(spec, staging.join("compiler"));
        let bundle = match cargo_ir::capture(&request)? {
            CaptureOutcome::Captured(bundle) => bundle,
            CaptureOutcome::Fresh { .. } => {
                let retry = self.build_request(spec, staging.join("compiler-retry"));
                let CaptureOutcome::Captured(bundle) = cargo_ir::capture(&retry)? else {
                    return Err(cargo_ir::Error::MissingEvidence.into());
                };

                bundle
            }
        };

        sources.validate()?;

        self.store.publish(
            capture_id.clone(),
            &request_key,
            spec,
            &bundle,
            &sources,
            selected_target(spec, &bundle.toolchain.host),
        )
    }

    /// Lists completed captures from newest to oldest.
    pub fn captures(&self) -> Result<Vec<CaptureSummary>> {
        self.store.captures()
    }

    /// Finds concrete instances in one completed capture.
    pub fn find(&self, capture_id: &CaptureId, query: &str) -> Result<FindResult> {
        self.store.find(capture_id, query)
    }

    /// Loads exact LLVM bodies and optional captured source for one instance.
    pub fn show(
        &self,
        capture_id: &CaptureId,
        instance_id: &InstanceId,
        include_source: bool,
    ) -> Result<ShowView> {
        let mut view = self.store.show(capture_id, instance_id)?;

        if include_source {
            let sources = self.store.sources(capture_id)?;
            view.source = find_item(&view.instance.definition, &sources);
        }

        Ok(view)
    }

    fn build_request(&self, spec: &BuildSpec, analysis_directory: PathBuf) -> BuildRequest {
        BuildRequest {
            workspace_root: self.workspace_root.clone(),
            manifest_path: spec.manifest_path.clone(),
            package: spec.package.clone(),
            target: spec.target.clone(),
            profile: spec.profile.clone(),
            features: spec.features.clone(),
            all_features: spec.all_features,
            no_default_features: spec.no_default_features,
            target_triple: spec.target_triple.clone(),
            locked: spec.locked,
            offline: spec.offline,
            frozen: spec.frozen,
            analysis_directory,
        }
    }
}

fn metadata(directory: &Path, manifest_path: Option<&Path>) -> Result<cargo_metadata::Metadata> {
    let mut command = cargo_metadata::MetadataCommand::new();
    command.current_dir(directory).no_deps();
    if let Some(path) = manifest_path {
        command.manifest_path(path);
    }

    Ok(command.exec()?)
}

fn request_key(
    spec: &BuildSpec,
    toolchain: &cargo_ir::Toolchain,
    target_directory: &Path,
    source_digest: blake3::Hash,
) -> Result<String> {
    let value = serde_json::json!({
        "spec": spec,
        "rustc_commit": toolchain.commit_hash,
        "target_directory": target_directory,
        "inputs": source_digest.to_hex().as_str(),
        "environment": compiler_environment(),
    });
    let encoded = serde_json::to_vec(&value)?;

    Ok(blake3::hash(&encoded).to_hex().to_string())
}

fn compiler_environment() -> Vec<(String, String)> {
    let mut environment = env::vars_os()
        .filter(|(name, _)| compiler_environment_name(name))
        .map(|(name, value)| {
            let digest = blake3::hash(value.to_string_lossy().as_bytes());
            (
                name.to_string_lossy().into_owned(),
                digest.to_hex().to_string(),
            )
        })
        .collect::<Vec<_>>();
    environment.sort();

    environment
}

fn compiler_environment_name(name: &OsString) -> bool {
    let name = name.to_string_lossy();

    matches!(
        name.as_ref(),
        "CARGO_ENCODED_RUSTFLAGS"
            | "CARGO"
            | "CARGO_HOME"
            | "RUSTFLAGS"
            | "RUSTC"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "RUSTUP_TOOLCHAIN"
    ) || name.starts_with("CARGO_BUILD_")
        || name.starts_with("CARGO_PROFILE_")
        || name.starts_with("CARGO_TARGET_")
}

fn selected_target<'a>(spec: &'a BuildSpec, host: &'a str) -> &'a str {
    spec.target_triple.as_deref().unwrap_or(host)
}

fn remove_staging(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|source| crate::Error::filesystem("remove", path, source))?;
    }

    Ok(())
}
