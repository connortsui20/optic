//! Coordinates compiler capture, persistence, lookup, and inspection.
//!
//! [`Application`] is the single product entry point. It serializes capture mutations while keeping
//! every completed query read-only and independent of shared client state.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_ir::BuildRequest;
use serde::Serialize;

use crate::source::{SourceBaseline, find_item};
use crate::store::{FileLock, Store, lock_workspace_exclusive, lock_workspace_shared};
use crate::{
    BuildSpec, CachePolicy, CaptureId, CaptureSummary, CleanSummary, CompilerOutput, FindResult,
    InstanceId, Result, ShowView,
};

const EVIDENCE_VERSION: u32 = 2;

/// Product workflows for one Cargo workspace and its `.optic` store.
pub struct Application {
    /// Prevents `clean` from removing the store while this application uses it.
    _operation_lock: FileLock,

    workspace_root: PathBuf,

    target_directory: PathBuf,

    store: Store,
}

impl Application {
    /// Opens the project store for the Cargo workspace containing `directory`.
    ///
    /// # Errors
    ///
    /// Returns an error if Cargo metadata, the workspace lock, or the evidence store is not
    /// available.
    pub fn discover(directory: &Path, manifest_path: Option<&Path>) -> Result<Self> {
        let metadata = metadata(directory, manifest_path)?;
        let workspace_root = metadata.workspace_root.into_std_path_buf();
        let target_directory = metadata.target_directory.into_std_path_buf();
        let operation_lock = lock_workspace_shared(&workspace_root)?;
        let store = Store::open(&workspace_root)?;

        Ok(Self {
            _operation_lock: operation_lock,
            workspace_root,
            target_directory,
            store,
        })
    }

    /// Removes the `.optic` cache for the Cargo workspace that contains `directory`.
    ///
    /// # Errors
    ///
    /// Returns an error if Cargo metadata, the workspace lock, or the cache path is not available.
    pub fn clean(directory: &Path, manifest_path: Option<&Path>) -> Result<CleanSummary> {
        let metadata = metadata(directory, manifest_path)?;
        let workspace_root = metadata.workspace_root.into_std_path_buf();
        let path = workspace_root.join(".optic");
        let _operation_lock = lock_workspace_exclusive(&workspace_root)?;
        let removed = remove_cache(&path)?;

        Ok(CleanSummary { path, removed })
    }

    /// Captures or reuses enriched compiler evidence for one Cargo target.
    ///
    /// # Errors
    ///
    /// Returns an error if source capture, compiler execution, or evidence publication fails.
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
        let summary = result?;
        cleanup?;

        Ok(summary)
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
        let bundle = cargo_ir::capture(&request)?;

        sources.validate()?;

        self.store.publish(
            capture_id,
            &request_key,
            spec,
            &bundle,
            &sources,
            selected_target(spec, &bundle.toolchain.host),
        )
    }

    /// Lists completed captures from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error if the evidence catalog cannot be read.
    pub fn captures(&self) -> Result<Vec<CaptureSummary>> {
        self.store.captures()
    }

    /// Finds concrete instances in one completed capture.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture selector is not unique or the catalog cannot be read.
    pub fn find(&self, capture_id: &CaptureId, query: &str) -> Result<FindResult> {
        self.store.find(capture_id, query)
    }

    pub(crate) fn unique_capture_prefix(&self, capture_id: &CaptureId) -> Result<CaptureId> {
        self.store.unique_capture_prefix(capture_id)
    }

    pub(crate) fn unique_instance_prefix(&self, instance_id: &InstanceId) -> Result<InstanceId> {
        self.store.unique_instance_prefix(instance_id)
    }

    /// Loads one compiler output and optional captured source for one instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the instance selector is not unique or its evidence cannot be read.
    pub fn show(
        &self,
        instance_id: &InstanceId,
        output: CompilerOutput,
        include_source: bool,
    ) -> Result<ShowView> {
        let mut view = self.store.show(instance_id, output)?;

        if include_source {
            let sources = self.store.sources(&view.capture_id)?;
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

fn remove_cache(path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(crate::Error::filesystem("read metadata for", path, source)),
    };
    let result = if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|source| crate::Error::filesystem("remove", path, source))?;

    Ok(true)
}

fn request_key(
    spec: &BuildSpec,
    toolchain: &cargo_ir::Toolchain,
    target_directory: &Path,
    source_digest: blake3::Hash,
) -> Result<String> {
    #[derive(Serialize)]
    struct CacheKey<'a> {
        evidence_version: u32,
        spec: &'a BuildSpec,
        rustc_commit: &'a str,
        target_directory: &'a Path,
        inputs: &'a str,
        environment: Vec<(String, String)>,
    }

    let inputs = source_digest.to_hex();
    let key = CacheKey {
        evidence_version: EVIDENCE_VERSION,
        spec,
        rustc_commit: &toolchain.commit_hash,
        target_directory,
        inputs: inputs.as_str(),
        environment: compiler_environment(),
    };
    let encoded = serde_json::to_vec(&key)?;

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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn cache_removal_does_not_follow_a_symbolic_link() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let retained = temporary.path().join("retained");
        let cache = temporary.path().join(".optic");
        fs::create_dir(&retained).expect("the test can create the retained directory");
        fs::write(retained.join("evidence"), b"retained")
            .expect("the test can create retained evidence");
        symlink(&retained, &cache).expect("the test can create the cache symbolic link");

        assert!(super::remove_cache(&cache).expect("cache removal succeeds"));
        assert!(!cache.exists());
        assert_eq!(
            fs::read(retained.join("evidence")).expect("the linked directory remains readable"),
            b"retained"
        );
    }
}
