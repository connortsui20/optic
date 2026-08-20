//! Coordinates compiler capture, persistence, lookup, and inspection.
//!
//! [`Application`] is the single product entry point. It serializes capture mutations while keeping
//! every completed query read-only and independent of shared client state.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_ir::BuildRequest;
use serde::Serialize;

use crate::source::{SourceBaseline, find_item_at};
use crate::store::{
    AnalysisKey, CaptureCacheKey, FileLock, LEGACY_STORE_ENTRIES, Store, lock_workspace_exclusive,
    lock_workspace_shared,
};
use crate::{
    BodySetDelta, BodySetSummary, BuildSpec, CachePolicy, CaptureDetails, CaptureId,
    CaptureSummary, CleanSummary, CompareView, CompilerOutput, FindResult, GcSummary, InstanceId,
    RemoveSummary, Result, ShowView, StoreStatus, VerifySummary,
};

const EVIDENCE_VERSION: u32 = 3;

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

    /// Removes stored evidence for the Cargo workspace that contains `directory`.
    ///
    /// # Errors
    ///
    /// Returns an error if Cargo metadata, the workspace lock, or an evidence path is not available.
    pub fn clean(directory: &Path, manifest_path: Option<&Path>) -> Result<CleanSummary> {
        let metadata = metadata(directory, manifest_path)?;
        let workspace_root = metadata.workspace_root.into_std_path_buf();
        let path = workspace_root.join(".optic").join("store");
        let _operation_lock = lock_workspace_exclusive(&workspace_root)?;
        let removed = remove_stored_evidence(&workspace_root)?;

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
        if spec.capture_profile != crate::CaptureProfile::Experiment
            && !spec.rustc_arguments.is_empty()
        {
            return Err(crate::Error::InvalidRequest {
                message: "--rustc-arg requires --evidence-profile experiment".to_owned(),
            });
        }

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
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
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
        let request_key = request_key(spec, toolchain, &self.target_directory)?;
        let cached = match cache_policy {
            CachePolicy::Reuse => self.store.cached_capture(&request_key)?,
            CachePolicy::Refresh => None,
        };
        let analysis_key = cached
            .as_ref()
            .map_or_else(AnalysisKey::new, |cached| cached.analysis_key.clone());
        let analysis_directory = self.store.analysis_directory(&analysis_key);
        prepare_analysis_directory(&analysis_directory)?;
        let sources = SourceBaseline::capture(&self.workspace_root, spec, staging)?;
        let request = self.build_request(spec, analysis_directory.clone());
        let outcome = cargo_ir::capture(&request);
        let result = match outcome {
            Ok(cargo_ir::CaptureOutcome::Captured { bundle }) => {
                sources.validate()?;
                self.store.publish(
                    capture_id,
                    CaptureCacheKey::new(&request_key, &analysis_key),
                    spec,
                    &bundle,
                    &sources,
                    selected_target(spec, &bundle.toolchain.host),
                )
            }
            Ok(cargo_ir::CaptureOutcome::Fresh { .. }) => {
                sources.validate()?;
                cached.map(|cached| cached.summary).ok_or_else(|| {
                    crate::Error::EvidenceUnavailable {
                        message: "Cargo reused the selected target, but Optic has no verified capture for this build. Run the same command with --fresh".to_owned(),
                    }
                })
            }
            Err(error) => Err(error.into()),
        };
        let cleanup = remove_analysis_directory(&analysis_directory);
        match (result, cleanup) {
            (Ok(summary), Ok(())) => Ok(summary),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Lists completed captures from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error if the evidence catalog cannot be read.
    pub fn captures(&self) -> Result<Vec<CaptureSummary>> {
        self.store.captures()
    }

    /// Returns the size and object counts for this workspace's evidence store.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalog or blob directory cannot be read.
    pub fn status(&self) -> Result<StoreStatus> {
        let _reader = self.store.lock_evidence_reader()?;

        self.store.status()
    }

    /// Removes one completed capture but leaves shared blobs for explicit garbage collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture selector is not unique or the catalog cannot be updated.
    pub fn remove(&mut self, capture_id: &CaptureId) -> Result<RemoveSummary> {
        let _writer = self.store.lock_writer()?;

        self.store.remove_capture(capture_id)
    }

    /// Removes content-addressed blobs that no completed capture references.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalog or blob directory cannot be updated.
    pub fn gc(&mut self) -> Result<GcSummary> {
        let _writer = self.store.lock_writer()?;

        self.store.gc()
    }

    /// Verifies the digest of every blob referenced by completed captures.
    ///
    /// # Errors
    ///
    /// Returns an error if referenced evidence is missing or corrupt.
    pub fn verify(&self) -> Result<VerifySummary> {
        let _reader = self.store.lock_evidence_reader()?;

        self.store.verify()
    }

    /// Returns reproducibility and artifact provenance for one completed capture.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture selector is not unique or its metadata is invalid.
    pub fn inspect(&self, capture_id: &CaptureId) -> Result<CaptureDetails> {
        self.store.capture_details(capture_id)
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
        let _reader = self.store.lock_evidence_reader()?;
        let mut view = self.store.show(instance_id, output)?;

        if include_source {
            view.source = if let Some(location) = &view.instance.source {
                self.store
                    .source_file(&view.capture_id, location)?
                    .and_then(|source| find_item_at(location, &source))
            } else {
                None
            };
        }

        Ok(view)
    }

    /// Compares compact LLVM structure for two exact instances.
    ///
    /// # Errors
    ///
    /// Returns an error if either instance or its capture evidence cannot be read.
    pub fn compare(
        &self,
        before: &InstanceId,
        after: &InstanceId,
        output: CompilerOutput,
    ) -> Result<CompareView> {
        let before_view = self.show(before, output, false)?;
        let after_view = self.show(after, output, false)?;
        let before_capture = self.store.capture_details(&before_view.capture_id)?;
        let after_capture = self.store.capture_details(&after_view.capture_id)?;
        let mut compatibility_differences = Vec::new();

        if before_capture.summary.rustc_release != after_capture.summary.rustc_release {
            compatibility_differences.push("rustc release".to_owned());
        }
        if before_capture.summary.llvm_version != after_capture.summary.llvm_version {
            compatibility_differences.push("LLVM version".to_owned());
        }
        if before_capture.summary.target != after_capture.summary.target {
            compatibility_differences.push("compiler target".to_owned());
        }
        if before_capture.summary.capture_profile != after_capture.summary.capture_profile {
            compatibility_differences.push("evidence profile".to_owned());
        }
        if before_capture.request != after_capture.request {
            compatibility_differences.push("Cargo build request".to_owned());
        }

        let before_summary = BodySetSummary::from_bodies(&before_view.bodies);
        let after_summary = BodySetSummary::from_bodies(&after_view.bodies);
        let delta = BodySetDelta::between(&before_summary, &after_summary);

        Ok(CompareView {
            output,
            before_instance: before_view.instance,
            after_instance: after_view.instance,
            compatibility_differences,
            before: before_summary,
            after: after_summary,
            delta,
        })
    }

    fn build_request(&self, spec: &BuildSpec, analysis_directory: PathBuf) -> BuildRequest {
        BuildRequest {
            workspace_root: self.workspace_root.clone(),
            manifest_path: spec.manifest_path.clone(),
            package: spec.package.clone(),
            target: spec.target.as_ref().map(|target| match target {
                crate::BuildTarget::Library => cargo_ir::CargoTarget::Library,
                crate::BuildTarget::Binary(name) => cargo_ir::CargoTarget::Binary(name.clone()),
                crate::BuildTarget::Benchmark(name) => {
                    cargo_ir::CargoTarget::Benchmark(name.clone())
                }
                crate::BuildTarget::Example(name) => cargo_ir::CargoTarget::Example(name.clone()),
            }),
            profile: spec.profile.clone(),
            features: spec.features.clone(),
            all_features: spec.all_features,
            no_default_features: spec.no_default_features,
            target_triple: spec.target_triple.clone(),
            locked: spec.locked,
            offline: spec.offline,
            frozen: spec.frozen,
            capture_profile: match spec.capture_profile {
                crate::CaptureProfile::Faithful => cargo_ir::CaptureProfile::Faithful,
                crate::CaptureProfile::Enriched => cargo_ir::CaptureProfile::Enriched,
                crate::CaptureProfile::Experiment => cargo_ir::CaptureProfile::Experiment {
                    rustc_arguments: spec.rustc_arguments.clone(),
                },
            },
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

fn remove_path(path: &Path) -> Result<bool> {
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

fn remove_stored_evidence(workspace_root: &Path) -> Result<bool> {
    let optic = workspace_root.join(".optic");
    let mut removed = remove_path(&optic.join("store"))?;

    for entry in LEGACY_STORE_ENTRIES {
        removed |= remove_path(&optic.join(entry))?;
    }

    removed |= remove_path(&workspace_root.join(".optic.lock"))?;

    Ok(removed)
}

fn request_key(
    spec: &BuildSpec,
    toolchain: &cargo_ir::Toolchain,
    target_directory: &Path,
) -> Result<String> {
    #[derive(Serialize)]
    struct CacheKey<'a> {
        evidence_version: u32,
        spec: &'a BuildSpec,
        rustc_commit: &'a str,
        target_directory: &'a Path,
        environment: Vec<(String, String)>,
    }

    let key = CacheKey {
        evidence_version: EVIDENCE_VERSION,
        spec,
        rustc_commit: &toolchain.commit_hash,
        target_directory,
        environment: compiler_environment(),
    };
    let encoded = serde_json::to_vec(&key)?;

    Ok(blake3::hash(&encoded).to_hex().to_string())
}

fn prepare_analysis_directory(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|source| crate::Error::filesystem("remove", path, source))?;
    }
    fs::create_dir_all(path).map_err(|source| crate::Error::filesystem("create", path, source))
}

fn remove_analysis_directory(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|source| crate::Error::filesystem("remove", path, source))?;
    }

    Ok(())
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

fn compiler_environment_name(name: &OsStr) -> bool {
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
    #[test]
    fn clean_removes_only_current_and_legacy_evidence() {
        use std::fs;

        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let optic = temporary.path().join(".optic");
        let operation_lock = optic.join("locks/operation.lock");
        let config = optic.join("config.toml");
        let unknown = optic.join("future-data");
        fs::create_dir_all(optic.join("store/blobs"))
            .expect("the test can create the current store");
        fs::create_dir_all(optic.join("blobs")).expect("the test can create legacy evidence");
        fs::create_dir_all(operation_lock.parent().expect("the lock has a parent"))
            .expect("the test can create the lock directory");
        fs::write(&operation_lock, b"lock").expect("the test can create the operation lock");
        fs::write(&config, b"config").expect("the test can create future configuration");
        fs::write(&unknown, b"unknown").expect("the test can create an unknown root entry");
        fs::write(temporary.path().join(".optic.lock"), b"legacy lock")
            .expect("the test can create the legacy lock");

        assert!(
            super::remove_stored_evidence(temporary.path()).expect("evidence removal succeeds")
        );
        assert!(!optic.join("store").exists());
        assert!(!optic.join("blobs").exists());
        assert!(!temporary.path().join(".optic.lock").exists());
        assert_eq!(
            fs::read(&operation_lock).expect("the lock remains"),
            b"lock"
        );
        assert_eq!(fs::read(&config).expect("configuration remains"), b"config");
        assert_eq!(
            fs::read(&unknown).expect("the unknown entry remains"),
            b"unknown"
        );
        assert!(optic.is_dir());

        assert!(
            !super::remove_stored_evidence(temporary.path())
                .expect("repeated evidence removal succeeds")
        );
    }

    #[cfg(unix)]
    #[test]
    fn clean_retains_the_operation_lock_inode() {
        use std::fs;
        use std::os::unix::fs::MetadataExt;

        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let lock = temporary.path().join(".optic/locks/operation.lock");
        fs::create_dir_all(lock.parent().expect("the lock has a parent"))
            .expect("the test can create the lock directory");
        fs::write(&lock, []).expect("the test can create the operation lock");
        fs::create_dir_all(temporary.path().join(".optic/store"))
            .expect("the test can create the store");
        let inode = fs::metadata(&lock)
            .expect("the operation lock has metadata")
            .ino();

        assert!(
            super::remove_stored_evidence(temporary.path()).expect("evidence removal succeeds")
        );
        assert_eq!(
            fs::metadata(&lock)
                .expect("the operation lock remains")
                .ino(),
            inode
        );
    }

    #[cfg(unix)]
    #[test]
    fn store_removal_does_not_follow_a_symbolic_link() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let retained = temporary.path().join("retained");
        let optic = temporary.path().join(".optic");
        let store = optic.join("store");
        fs::create_dir(&optic).expect("the test can create the Optic directory");
        fs::create_dir(&retained).expect("the test can create the retained directory");
        fs::write(retained.join("evidence"), b"retained")
            .expect("the test can create retained evidence");
        symlink(&retained, &store).expect("the test can create the store symbolic link");

        assert!(super::remove_stored_evidence(temporary.path()).expect("store removal succeeds"));
        assert!(!store.exists());
        assert_eq!(
            fs::read(retained.join("evidence")).expect("the linked directory remains readable"),
            b"retained"
        );
    }
}
