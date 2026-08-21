//! Coordinates compiler capture, persistence, lookup, and inspection.
//!
//! [`Application`] is the single product entry point. It serializes capture mutations while keeping
//! every completed query read-only and independent of shared client state.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use cargo_ir::BuildRequest;
use serde::Serialize;

use crate::pending::{PendingCapture, ResumableCapture};
use crate::source::{SourceBaseline, find_item_at};
use crate::store::{
    AnalysisKey, CaptureCacheKey, FileLock, LEGACY_STORE_ENTRIES, Store, lock_workspace_exclusive,
    lock_workspace_shared,
};
use crate::{
    BodySetDelta, BodySetSummary, BuildSpec, CachePolicy, CaptureDetails, CaptureDisposition,
    CaptureEvent, CaptureId, CapturePhase, CaptureSummary, CleanSummary, CompareView,
    CompilerOutput, FindOptions, FindResult, GcSummary, InspectEvent, InspectSummary, InstanceId,
    RemarkOptions, RemarkShowView, RemoveSummary, Result, ShowEvent, ShowSummary, ShowView,
    StoreStatus, StreamCount, VerifySummary,
};

const EVIDENCE_VERSION: u32 = 5;

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

    /// Captures or reuses compiler evidence and discards Cargo output events.
    ///
    /// # Errors
    ///
    /// Returns an error from [`Self::capture_with_events`].
    pub fn capture(
        &mut self,
        spec: &BuildSpec,
        cache_policy: CachePolicy,
    ) -> Result<CaptureSummary> {
        self.capture_with_events(spec, cache_policy, |_| ControlFlow::Continue(()))
    }

    /// Captures or reuses evidence and reports user-visible Cargo output as it arrives.
    ///
    /// The callback runs on the calling thread. It does not report the internal freshness probe for
    /// retained pending evidence.
    ///
    /// # Errors
    ///
    /// Returns an error if source capture, compiler execution, or evidence publication fails.
    pub fn capture_with_events(
        &mut self,
        spec: &BuildSpec,
        cache_policy: CachePolicy,
        mut on_event: impl FnMut(CaptureEvent) -> ControlFlow<()>,
    ) -> Result<CaptureSummary> {
        if spec.capture_profile != crate::CaptureProfile::Experiment
            && !spec.rustc_arguments.is_empty()
        {
            return Err(crate::Error::InvalidRequest {
                message: "--rustc-arg requires --evidence-profile experiment".to_owned(),
            });
        }

        let _writer = self.store.lock_writer()?;
        let toolchain = cargo_ir::inspect_workspace_toolchain(&self.workspace_root)?;
        let request_key = request_key(spec, &toolchain, &self.target_directory)?;
        let pending_directory = self.store.pending_directory(&request_key)?;

        if PendingCapture::exists(&pending_directory) {
            let request_template = self.build_request(spec, PathBuf::new());
            match PendingCapture::resume(
                &pending_directory,
                &request_key,
                spec,
                &request_template,
                &toolchain,
                true,
            ) {
                Ok(pending) => {
                    return self.resume_capture(
                        spec,
                        &request_key,
                        &pending_directory,
                        pending,
                        CaptureDisposition::Resumed,
                        &mut on_event,
                    );
                }
                Err(crate::Error::InputChanged { .. } | crate::Error::PendingInputsChanged) => {
                    remove_pending(&pending_directory)?;
                }
                Err(error) => return Err(error),
            }
        }

        remove_pending(&pending_directory)?;
        let cached = match cache_policy {
            CachePolicy::Reuse => self.store.cached_capture(&request_key)?,
            CachePolicy::Refresh => None,
        };
        let analysis_key = cached
            .as_ref()
            .map_or_else(AnalysisKey::new, |cached| cached.analysis_key.clone());
        let run_directory = pending_directory.join(analysis_key.as_str());
        let staging = run_directory.join("staging");
        let analysis_directory = run_directory.join("analysis");
        fs::create_dir_all(&staging)
            .map_err(|source| crate::Error::filesystem("create", &staging, source))?;
        emit_discardable_capture_event(
            &mut on_event,
            CaptureEvent::PhaseStarted(CapturePhase::Source),
            &pending_directory,
        )?;
        let sources = SourceBaseline::capture(&self.workspace_root, spec, &staging)?;
        emit_discardable_capture_event(
            &mut on_event,
            CaptureEvent::PhaseFinished(CapturePhase::Source),
            &pending_directory,
        )?;
        let request = self.build_request(spec, analysis_directory.clone());
        emit_discardable_capture_event(
            &mut on_event,
            CaptureEvent::PhaseStarted(CapturePhase::Compile),
            &pending_directory,
        )?;
        let outcome =
            cargo_ir::compile_with_events(&request, |event| on_event(CaptureEvent::Cargo(event)));
        if !matches!(outcome, Err(cargo_ir::Error::ConsumerStopped)) {
            emit_discardable_capture_event(
                &mut on_event,
                CaptureEvent::PhaseFinished(CapturePhase::Compile),
                &pending_directory,
            )?;
        }
        match outcome {
            Ok(cargo_ir::CompileOutcome::Compiled { compilation }) => {
                sources.validate()?;
                cargo_ir::require_compiled_evidence(&request)?;
                let capture_id = CaptureId::new();
                let marker = PendingCapture::new(
                    &request_key,
                    capture_id,
                    &analysis_key,
                    spec.clone(),
                    *compilation,
                    sources.pending()?,
                );
                marker.write(&pending_directory)?;
                let pending = PendingCapture::resume(
                    &pending_directory,
                    &request_key,
                    spec,
                    &self.build_request(spec, PathBuf::new()),
                    &toolchain,
                    false,
                )?;

                self.resume_capture(
                    spec,
                    &request_key,
                    &pending_directory,
                    pending,
                    CaptureDisposition::Captured,
                    &mut on_event,
                )
            }
            Ok(cargo_ir::CompileOutcome::Fresh { .. }) => {
                sources.validate()?;
                remove_pending(&pending_directory)?;

                cached.map(|cached| cached.summary).ok_or_else(|| {
                    crate::Error::EvidenceUnavailable {
                        message: "Cargo reused the selected target, but Optic has no verified capture for this build. Run the same command with --fresh".to_owned(),
                    }
                })
            }
            Err(error) => {
                remove_pending(&pending_directory)?;
                if matches!(error, cargo_ir::Error::ConsumerStopped) {
                    Err(crate::Error::ConsumerStopped)
                } else {
                    Err(error.into())
                }
            }
        }
    }

    fn resume_capture(
        &mut self,
        spec: &BuildSpec,
        request_key: &str,
        pending_directory: &Path,
        pending: ResumableCapture,
        disposition: CaptureDisposition,
        on_event: &mut impl FnMut(CaptureEvent) -> ControlFlow<()>,
    ) -> Result<CaptureSummary> {
        if let Some(summary) = self.store.completed_capture(
            &pending.capture_id,
            request_key,
            &pending.analysis_key,
            CaptureDisposition::Resumed,
        )? {
            remove_pending(pending_directory)?;

            return Ok(summary);
        }

        let target = selected_target(spec, &pending.compilation.toolchain.host).to_owned();
        emit_capture_event(on_event, CaptureEvent::PhaseStarted(CapturePhase::Ingest))?;
        let mut summary = self.store.publish_stream(
            &pending.capture_id,
            CaptureCacheKey::new(request_key, &pending.analysis_key),
            spec,
            pending.compilation,
            &pending.sources,
            &target,
        )?;
        let finish =
            emit_capture_event(on_event, CaptureEvent::PhaseFinished(CapturePhase::Ingest));
        summary.disposition = disposition;
        remove_pending(pending_directory)?;
        finish?;

        Ok(summary)
    }

    /// Lists completed captures from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error if the evidence catalog cannot be read.
    pub fn captures(&self) -> Result<Vec<CaptureSummary>> {
        self.store.captures()
    }

    /// Streams completed captures from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalog is invalid or the consumer stops.
    pub fn captures_with_events(
        &self,
        mut on_capture: impl FnMut(CaptureSummary) -> std::ops::ControlFlow<()>,
    ) -> Result<StreamCount> {
        let items = self.store.stream_captures(&mut on_capture)?;

        Ok(StreamCount { items })
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

    /// Streams reproducibility metadata and artifact provenance for one completed capture.
    ///
    /// The callback runs on the calling thread. Returning [`ControlFlow::Break`] stops the read.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture selector or stored metadata is invalid, or the consumer
    /// stops.
    pub fn inspect_with_events(
        &self,
        capture_id: &CaptureId,
        mut on_event: impl FnMut(InspectEvent) -> ControlFlow<()>,
    ) -> Result<InspectSummary> {
        let metadata = self.store.capture_metadata(capture_id)?;
        let resolved_id = metadata.summary.id.clone();
        emit_inspect_event(
            &mut on_event,
            InspectEvent::Started {
                metadata: Box::new(metadata.clone()),
            },
        )?;
        let artifacts = self
            .store
            .stream_capture_artifacts(&resolved_id, &mut |artifact| {
                on_event(InspectEvent::Artifact { artifact })
            })?;
        let remark_files = self
            .store
            .stream_capture_remark_files(&resolved_id, &mut |remark_file| {
                on_event(InspectEvent::RemarkFile { remark_file })
            })?;

        Ok(InspectSummary {
            capture_id: resolved_id,
            artifacts,
            remark_files,
        })
    }

    /// Finds concrete instances in one completed capture.
    ///
    /// # Errors
    ///
    /// Returns an error if the options are invalid, the capture selector is not unique, or the
    /// catalog cannot be read.
    pub fn find(&self, capture_id: &CaptureId, options: &FindOptions) -> Result<FindResult> {
        validate_find_options(options)?;

        self.store.find(capture_id, options)
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

    /// Streams one compiler output and optional captured source for one instance.
    ///
    /// The callback runs on the calling thread. Returning [`std::ops::ControlFlow::Break`] stops
    /// the read.
    ///
    /// # Errors
    ///
    /// Returns an error if the instance or stored evidence is invalid, or the consumer stops.
    pub fn show_with_events(
        &self,
        instance_id: &InstanceId,
        output: CompilerOutput,
        include_source: bool,
        mut on_event: impl FnMut(ShowEvent) -> std::ops::ControlFlow<()>,
    ) -> Result<ShowSummary> {
        let _reader = self.store.lock_evidence_reader()?;
        let show = self.store.prepare_show(instance_id, output)?;
        emit_show_event(
            &mut on_event,
            ShowEvent::Started {
                capture_id: show.capture_id.clone(),
                instance: show.instance.clone(),
                output,
            },
        )?;
        let has_source = if include_source {
            self.store.stream_show_source(&show, &mut on_event)?
        } else {
            false
        };
        let bodies = self.store.stream_show_bodies(&show, &mut on_event)?;

        Ok(ShowSummary {
            capture_id: show.capture_id,
            instance: show.instance,
            output,
            bodies,
            source: has_source,
        })
    }

    /// Loads optimization remarks and optional captured source for one instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the options, instance, stored records, or source evidence are invalid.
    pub fn show_remarks(
        &self,
        instance_id: &InstanceId,
        options: &RemarkOptions,
        include_source: bool,
    ) -> Result<RemarkShowView> {
        validate_remark_options(options)?;
        let _reader = self.store.lock_evidence_reader()?;
        let mut view = self.store.show_remarks(instance_id, options)?;

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
        let (before_result, before_summary) = self.summarize_instance(before, output)?;
        let (after_result, after_summary) = self.summarize_instance(after, output)?;
        let before_capture = self.store.capture_details(&before_result.capture_id)?;
        let after_capture = self.store.capture_details(&after_result.capture_id)?;
        let mut compatibility_differences = Vec::new();

        if before_capture.summary.rustc_release != after_capture.summary.rustc_release {
            compatibility_differences.push("rustc release".to_owned());
        }
        if before_capture.compiler.commit_hash != after_capture.compiler.commit_hash {
            compatibility_differences.push("rustc commit".to_owned());
        }
        if before_capture.compiler.host != after_capture.compiler.host {
            compatibility_differences.push("compiler host".to_owned());
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
        if effective_compiler_environment(&before_capture.environment)
            != effective_compiler_environment(&after_capture.environment)
        {
            compatibility_differences.push("compiler environment".to_owned());
        }
        if user_wrapper_chain(&before_capture.wrapper_chain)
            != user_wrapper_chain(&after_capture.wrapper_chain)
        {
            compatibility_differences.push("compiler wrappers".to_owned());
        }
        if effective_rustc_arguments(before_capture.rustc.as_ref())
            != effective_rustc_arguments(after_capture.rustc.as_ref())
        {
            compatibility_differences.push("rustc arguments".to_owned());
        }

        let delta = BodySetDelta::between(&before_summary, &after_summary);

        Ok(CompareView {
            output,
            before_instance: before_result.instance,
            after_instance: after_result.instance,
            compatibility_differences,
            before: before_summary,
            after: after_summary,
            delta,
        })
    }

    fn summarize_instance(
        &self,
        instance_id: &InstanceId,
        output: CompilerOutput,
    ) -> Result<(ShowSummary, BodySetSummary)> {
        let mut body_set = BodySetSummary::empty();
        let result = self.show_with_events(instance_id, output, false, |event| {
            if let ShowEvent::BodyFinished { summary } = event {
                body_set.add_body(&summary);
            }

            std::ops::ControlFlow::Continue(())
        })?;

        Ok((result, body_set))
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
            capture_remarks: spec.capture_remarks,
            analysis_directory,
        }
    }
}

fn emit_show_event(
    on_event: &mut impl FnMut(ShowEvent) -> std::ops::ControlFlow<()>,
    event: ShowEvent,
) -> Result<()> {
    if on_event(event).is_break() {
        return Err(crate::Error::ConsumerStopped);
    }

    Ok(())
}

fn emit_capture_event(
    on_event: &mut impl FnMut(CaptureEvent) -> ControlFlow<()>,
    event: CaptureEvent,
) -> Result<()> {
    if on_event(event).is_break() {
        return Err(crate::Error::ConsumerStopped);
    }

    Ok(())
}

fn emit_discardable_capture_event(
    on_event: &mut impl FnMut(CaptureEvent) -> ControlFlow<()>,
    event: CaptureEvent,
    pending_directory: &Path,
) -> Result<()> {
    if let Err(error) = emit_capture_event(on_event, event) {
        remove_pending(pending_directory)?;

        return Err(error);
    }

    Ok(())
}

fn emit_inspect_event(
    on_event: &mut impl FnMut(InspectEvent) -> ControlFlow<()>,
    event: InspectEvent,
) -> Result<()> {
    if on_event(event).is_break() {
        return Err(crate::Error::ConsumerStopped);
    }

    Ok(())
}

pub(crate) fn validate_remark_options(options: &RemarkOptions) -> Result<()> {
    if options.pass.as_ref().is_some_and(String::is_empty) {
        return Err(crate::Error::InvalidRequest {
            message: "remark pass must not be empty, got an empty pass".to_owned(),
        });
    }
    if !(1..=RemarkOptions::MAX_LIMIT).contains(&options.limit) {
        return Err(crate::Error::InvalidRequest {
            message: format!(
                "remark limit must be from 1 through {}, got {}",
                RemarkOptions::MAX_LIMIT,
                options.limit
            ),
        });
    }

    Ok(())
}

fn validate_find_options(options: &FindOptions) -> Result<()> {
    if options.query.is_empty() {
        return Err(crate::Error::InvalidRequest {
            message: "find requires a non-empty query, got an empty query".to_owned(),
        });
    }
    if options.query.contains('\0') {
        return Err(crate::Error::InvalidRequest {
            message: "find query must not contain NUL, got a query with NUL".to_owned(),
        });
    }
    if !(1..=FindOptions::MAX_LIMIT).contains(&options.limit) {
        return Err(crate::Error::InvalidRequest {
            message: format!(
                "find limit must be from 1 through {}, got {}",
                FindOptions::MAX_LIMIT,
                options.limit
            ),
        });
    }

    Ok(())
}

fn metadata(directory: &Path, manifest_path: Option<&Path>) -> Result<cargo_metadata::Metadata> {
    let mut command = cargo_metadata::MetadataCommand::new();
    // NB: MetadataCommand cannot remove inherited variables. An empty value disables unstable
    // access for rustc probes that Cargo metadata can start.
    command
        .current_dir(directory)
        .no_deps()
        .env("RUSTC_BOOTSTRAP", "");
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
        toolchain: &'a cargo_ir::Toolchain,
        target_directory: &'a Path,
        environment: Vec<(String, String)>,
    }

    let key = CacheKey {
        evidence_version: EVIDENCE_VERSION,
        spec,
        toolchain,
        target_directory,
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

fn effective_compiler_environment(environment: &[crate::EnvironmentView]) -> Vec<(&str, &str)> {
    // Older queryable captures can contain the ambient value that new captures clear.
    environment
        .iter()
        .filter(|variable| variable.name != "RUSTC_BOOTSTRAP")
        .map(|variable| (variable.name.as_str(), variable.value.as_str()))
        .collect()
}

fn user_wrapper_chain(wrappers: &[String]) -> &[String] {
    // The Optic driver is always the outer wrapper and does not affect captured code generation.
    wrappers.get(1..).unwrap_or_default()
}

/// Removes Optic evidence-only arguments from a recorded rustc command.
fn effective_rustc_arguments(command: Option<&crate::CommandView>) -> Option<Vec<&str>> {
    let arguments = &command?.arguments;
    let mut effective = Vec::with_capacity(arguments.len());
    let mut index = 0;

    while index < arguments.len() {
        if index + 1 < arguments.len()
            && is_evidence_collection_argument_pair(&arguments[index], &arguments[index + 1])
        {
            index += 2;

            continue;
        }
        if is_joined_evidence_collection_argument(&arguments[index]) {
            index += 1;

            continue;
        }

        effective.push(arguments[index].as_str());
        index += 1;
    }

    Some(effective)
}

fn is_evidence_collection_argument_pair(option: &str, value: &str) -> bool {
    matches!((option, value), ("-C", "save-temps") | ("-C", "remark=all"))
        || option == "-Z" && (value.starts_with("temps-dir=") || value.starts_with("remark-dir="))
}

fn is_joined_evidence_collection_argument(argument: &str) -> bool {
    matches!(argument, "-Csave-temps" | "-Cremark=all")
        || argument.starts_with("-Ztemps-dir=")
        || argument.starts_with("-Zremark-dir=")
}

fn selected_target<'a>(spec: &'a BuildSpec, host: &'a str) -> &'a str {
    spec.target_triple.as_deref().unwrap_or(host)
}

fn remove_pending(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|source| crate::Error::filesystem("remove", path, source))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{
        compiler_environment_name, effective_compiler_environment, effective_rustc_arguments,
        user_wrapper_chain,
    };
    use crate::{CommandView, EnvironmentView, Error, FindOptions, RemarkOptions};

    #[test]
    fn compatibility_keeps_codegen_arguments_and_ignores_evidence_collection_arguments() {
        let command = CommandView {
            program: "rustc".to_owned(),
            arguments: [
                "--crate-name",
                "example",
                "-C",
                "save-temps",
                "-Ztemps-dir=/optic/analysis",
                "-C",
                "target-cpu=native",
                "-Cremark=all",
                "-Z",
                "remark-dir=/optic/remarks",
            ]
            .map(str::to_owned)
            .to_vec(),
        };

        assert_eq!(
            effective_rustc_arguments(Some(&command)),
            Some(vec!["--crate-name", "example", "-C", "target-cpu=native"])
        );
    }

    #[test]
    fn compatibility_uses_only_the_user_wrapper_chain() {
        let wrappers = ["optic-driver", "sccache", "workspace-wrapper"].map(str::to_owned);

        assert_eq!(
            user_wrapper_chain(&wrappers),
            ["sccache", "workspace-wrapper"]
        );
        assert!(user_wrapper_chain(&[]).is_empty());
    }

    #[test]
    fn ambient_bootstrap_is_not_an_effective_compiler_setting() {
        let environment = vec![
            EnvironmentView {
                name: "RUSTC_BOOTSTRAP".to_owned(),
                value: "1".to_owned(),
            },
            EnvironmentView {
                name: "RUSTFLAGS".to_owned(),
                value: "-C target-cpu=native".to_owned(),
            },
        ];

        assert_eq!(
            effective_compiler_environment(&environment),
            vec![("RUSTFLAGS", "-C target-cpu=native")]
        );
        assert!(!compiler_environment_name(OsStr::new("RUSTC_BOOTSTRAP")));
    }

    #[test]
    fn find_options_reject_nul_and_out_of_range_limits() {
        assert!(matches!(
            super::validate_find_options(&FindOptions::new("kernel\0suffix")),
            Err(Error::InvalidRequest { .. })
        ));
        let mut options = FindOptions::new("kernel");
        options.limit = 0;
        assert!(matches!(
            super::validate_find_options(&options),
            Err(Error::InvalidRequest { .. })
        ));
        options.limit = FindOptions::MAX_LIMIT + 1;
        assert!(matches!(
            super::validate_find_options(&options),
            Err(Error::InvalidRequest { .. })
        ));
    }

    #[test]
    fn remark_options_reject_empty_passes_and_out_of_range_limits() {
        let mut options = RemarkOptions {
            pass: Some(String::new()),
            ..RemarkOptions::default()
        };
        assert!(matches!(
            super::validate_remark_options(&options),
            Err(Error::InvalidRequest { .. })
        ));
        options.pass = None;
        options.limit = 0;
        assert!(matches!(
            super::validate_remark_options(&options),
            Err(Error::InvalidRequest { .. })
        ));
        options.limit = RemarkOptions::MAX_LIMIT + 1;
        assert!(matches!(
            super::validate_remark_options(&options),
            Err(Error::InvalidRequest { .. })
        ));
    }

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
