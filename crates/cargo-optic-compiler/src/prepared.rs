//! Prepares one request identity and separates stopped probing from new collection.
//!
//! The request key partitions candidate captures. Cargo alone decides whether tracked inputs are
//! fresh. Published tokens are used only by probes, which stop if Cargo requests compilation.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use cargo_metadata::PackageId;
use cargo_metadata::Target;
use optic_records::AnalysisToken;
use optic_records::BuildRecord;
use optic_records::CaptureAnalysis;
use optic_records::CaptureKey;
use optic_records::CompilerIdentity;
use optic_records::TargetRecord;

use crate::BuildRequest;
use crate::CollectedBuild;
use crate::Error;
use crate::Workspace;
use crate::attempt::CargoAttempt;
use crate::build::cargo_arguments;
use crate::build::resolve_package;
use crate::build::resolve_target;
use crate::digest::request_key;
use crate::driver::RustcDriver;
use crate::driver::cargo_home;
use crate::error::invalid_environment;
use crate::manifest::read_manifest;
use crate::protocol;
use crate::toolchain::CompilerContext;

/// Cargo's result for a candidate whose analysis token was never compiled by the probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    /// Cargo positively reported one matching fresh selected artifact.
    Fresh,
    /// The selected wrapper verified its invocation and stopped before compiler analysis.
    Stale,
}

/// A resolved request and cached exact-version driver for one probe/collection operation.
///
/// The caller must serialize captures and driver provisioning. Cargo configuration and workspace
/// locations must remain fixed during this operation. Store initialization must precede probing or
/// collection. Preparation performs metadata discovery and driver provisioning only.
pub struct PreparedBuild<'a> {
    workspace: &'a Workspace,
    build: BuildRecord,
    compiler: CompilerIdentity,
    package: PackageId,
    /// The canonical selected package root, not the broader workspace root.
    package_root: PathBuf,
    target: Target,
    driver: RustcDriver,
    request_key: CaptureKey,
    /// Preserves a stopped probe's diagnostics until the subsequent collection succeeds or fails.
    stopped_probe: Option<CargoAttempt>,
}

/// Resolves the request and provisions its driver without compiling the selected Cargo target.
///
/// # Errors
///
/// Returns an error for unsupported compiler configuration, unresolved selection, missing compiler
/// components, an invalid driver cache entry, or a failed Cargo/compiler identity query.
pub fn prepare_build<'a>(
    workspace: &'a Workspace,
    request: &BuildRequest,
) -> Result<PreparedBuild<'a>, Error> {
    let metadata = workspace.read_metadata(request)?;
    let package = resolve_package(&metadata, request.package())?;
    let target = resolve_target(package, request.target())?.clone();
    let package_root = fs::canonicalize(
        package
            .manifest_path
            .parent()
            .expect("Cargo manifests have parent directories"),
    )
    .map_err(|source| Error::Filesystem {
        operation: "resolve selected package root",
        path: package.manifest_path.clone().into(),
        source,
    })?;

    let compiler = CompilerContext::discover(workspace)?;
    let driver = RustcDriver::provision(workspace, compiler.identity())?;

    if compiler.wrappers_configured() {
        eprintln!(
            "warning: Cargo Optic does not support configured rustc wrappers; \
             disabling them for this capture"
        );
        eprintln!("warning: the captured compiler output can differ from a normal wrapped build");
    }

    let build = BuildRecord::new(
        package.name.to_string(),
        package.version.to_string(),
        TargetRecord::new(target.name.clone(), request.target().kind())?,
        request.profile(),
        workspace.cargo().to_owned(),
        workspace.invocation_directory().to_owned(),
        cargo_arguments(request),
    )?;

    let output = Command::new(workspace.cargo())
        .current_dir(workspace.invocation_directory())
        .arg("-vV")
        .output()
        .map_err(|source| Error::StartProcess {
            program: workspace.cargo().to_owned(),
            source,
        })?;

    if !output.status.success() {
        return Err(Error::ProcessFailed {
            program: workspace.cargo().to_owned(),
            status: output.status.to_string(),
            diagnostics: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }

    let home = fs::canonicalize(cargo_home(workspace)?).map_err(|source| Error::Filesystem {
        operation: "resolve Cargo home",
        path: workspace.invocation_directory().to_owned(),
        source,
    })?;

    let request_key = request_key(
        &build,
        &package.id,
        &target,
        &metadata,
        &output.stdout,
        driver.key(),
        &home,
    )?;

    Ok(PreparedBuild {
        workspace,
        build,
        compiler: compiler.identity().clone(),
        package: package.id.clone(),
        package_root,
        target,
        driver,
        request_key,
        stopped_probe: None,
    })
}

impl PreparedBuild<'_> {
    /// Returns the conservative request partition, which does not establish Cargo freshness.
    pub fn request_key(&self) -> &CaptureKey {
        &self.request_key
    }

    /// Probes a complete candidate without compiling its published analysis token.
    ///
    /// A stale result retains diagnostics for the following [`Self::collect`] call. The caller must
    /// validate the candidate's stored evidence before probing it.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched candidate, a probe after a stopped probe, an unclassified
    /// Cargo failure, conflicting receipts, or success without one affirmative matching fresh
    /// artifact.
    pub fn probe(&mut self, candidate: &CaptureAnalysis) -> Result<Freshness, Error> {
        if candidate.request_key() != &self.request_key {
            return Err(invalid_environment(
                "candidate request key must match the prepared request, got another key",
            ));
        }

        if self.stopped_probe.is_some() {
            return Err(invalid_environment(
                "a stopped probe requires collection, got another probe",
            ));
        }

        let marker = selected_target_marker(candidate.token());
        let temporary = temporary_directory()?;
        let mut command = self.command(&marker, temporary.path())?;
        command.env(protocol::MODE_ENV, "probe");

        let mut attempt = CargoAttempt::run(command, temporary, &self.package, &self.target)?;
        let result = self.classify_probe(&attempt, candidate, &marker);

        match result {
            Ok(Freshness::Stale) => {
                attempt.relay_warnings()?;
                self.stopped_probe = Some(attempt);

                Ok(Freshness::Stale)
            }
            result => {
                attempt.replay()?;

                result
            }
        }
    }

    /// Collects once with a new token, even when an earlier variant remains fresh in Cargo.
    ///
    /// # Errors
    ///
    /// Returns an error unless Cargo completes successfully with one newly compiled selected
    /// artifact and the selected driver produces a complete manifest for this token.
    pub fn collect(mut self) -> Result<CollectedBuild, Error> {
        let result = self.collect_new_token();

        if result.is_err()
            && let Some(probe) = &mut self.stopped_probe
        {
            probe.replay()?;
        }

        result
    }

    fn collect_new_token(&self) -> Result<CollectedBuild, Error> {
        let token = AnalysisToken::generate();
        let marker = selected_target_marker(&token);
        let temporary = temporary_directory()?;
        let mut command = self.command(&marker, temporary.path())?;
        command.env(protocol::MODE_ENV, "collect");

        let mut attempt = CargoAttempt::run(command, temporary, &self.package, &self.target)?;
        attempt.replay()?;

        if !attempt.status.success() {
            return Err(attempt.failure(self.workspace.cargo()));
        }

        if verified_stale_receipt(&attempt.path("stale"), &marker)? {
            return Err(invalid_environment(
                "collection requires no stale receipt, got a stopped probe receipt",
            ));
        }

        let artifact = attempt.observation.completed(&self.target, false)?;
        let manifest = read_manifest(&attempt.path("manifest"), &marker)?;
        let evidence = crate::artifacts::collect(manifest, &self.compiler, &attempt.path(""))?;
        let analysis = CaptureAnalysis::new(self.request_key.clone(), token, artifact);

        Ok(CollectedBuild::new(
            self.build.clone(),
            self.compiler.clone(),
            evidence,
            analysis,
            attempt.into_temporary(),
        ))
    }

    fn classify_probe(
        &self,
        attempt: &CargoAttempt,
        candidate: &CaptureAnalysis,
        marker: &str,
    ) -> Result<Freshness, Error> {
        let stale = verified_stale_receipt(&attempt.path("stale"), marker)?;

        if attempt
            .path("manifest")
            .try_exists()
            .map_err(|source| Error::Filesystem {
                operation: "inspect probe manifest",
                path: attempt.path("manifest"),
                source,
            })?
        {
            return Err(invalid_environment(
                "probe requires no selected analysis manifest, got a compiler manifest",
            ));
        }

        if !attempt.status.success() {
            if stale && attempt.observation.selected.is_empty() {
                return Ok(Freshness::Stale);
            }

            return Err(attempt.failure(self.workspace.cargo()));
        }

        if stale {
            return Err(invalid_environment(
                "successful probe requires no stale receipt, got a stopped invocation",
            ));
        }

        let artifact = attempt.observation.completed(&self.target, true)?;

        if &artifact != candidate.artifact() {
            return Err(invalid_environment(
                "fresh selected artifact must match the stored observation, \
                 got different Cargo artifact fields",
            ));
        }

        Ok(Freshness::Fresh)
    }

    fn command(&self, marker: &str, directory: &Path) -> Result<Command, Error> {
        let source =
            fs::canonicalize(&self.target.src_path).map_err(|source| Error::Filesystem {
                operation: "resolve selected source",
                path: self.target.src_path.clone().into(),
                source,
            })?;

        let mut command = Command::new(self.workspace.cargo());
        command
            .current_dir(self.workspace.invocation_directory())
            .args(self.build.cargo_arguments())
            .arg("--message-format=json")
            .arg("--")
            .arg(marker)
            .env("RUSTC", self.compiler.rustc())
            .env(protocol::RUSTC_ENV, self.compiler.rustc())
            .env(protocol::SOURCE_ENV, source)
            .env(protocol::PACKAGE_ROOT_ENV, &self.package_root)
            .env(protocol::CRATE_NAME_ENV, self.target.name.replace('-', "_"))
            .env(protocol::STALE_RECEIPT_ENV, directory.join("stale"));
        self.driver
            .configure(&mut command, marker, &directory.join("manifest"));

        Ok(command)
    }
}

fn temporary_directory() -> Result<tempfile::TempDir, Error> {
    tempfile::Builder::new()
        .prefix("cargo-optic-compiler-")
        .tempdir()
        .map_err(|source| Error::Filesystem {
            operation: "create compiler attempt below",
            path: env::temp_dir(),
            source,
        })
}

fn selected_target_marker(token: &AnalysisToken) -> String {
    format!("{}\"{}\"", protocol::MARKER_PREFIX, token.as_str())
}

/// Accepts only this attempt's exact receipt, with an absent file returning false.
///
/// A present receipt must be a regular file with the exact protocol header and token. Wrong file
/// kinds, malformed bytes, and filesystem failures return errors instead of an absent receipt.
fn verified_stale_receipt(path: &Path, marker: &str) -> Result<bool, Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(Error::Filesystem {
                operation: "inspect stale receipt",
                path: path.to_owned(),
                source,
            });
        }
    };

    let expected = protocol::header(marker);

    if !metadata.is_file() || metadata.len() != expected.len() as u64 {
        return Err(invalid_environment(
            "stale receipt must contain exactly this attempt's header, got an invalid file",
        ));
    }

    let actual = fs::read(path).map_err(|source| Error::Filesystem {
        operation: "read stale receipt",
        path: path.to_owned(),
        source,
    })?;

    if actual != expected {
        return Err(invalid_environment(
            "stale receipt must match this analysis token and protocol, got a different header",
        ));
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use optic_records::AnalysisToken;

    use super::selected_target_marker;
    use super::verified_stale_receipt;
    use crate::protocol;

    #[test]
    fn stale_receipts_require_the_exact_protocol_and_token() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("stale");
        let marker = selected_target_marker(&AnalysisToken::generate());

        assert!(!verified_stale_receipt(&path, &marker).unwrap());

        fs::write(&path, protocol::header(&marker)).unwrap();
        assert!(verified_stale_receipt(&path, &marker).unwrap());

        let other = selected_target_marker(&AnalysisToken::generate());
        assert!(verified_stale_receipt(&path, &other).is_err());

        let mut wrong_version = protocol::header(&marker);
        wrong_version[protocol::MANIFEST_MAGIC.len()] = 0;
        let mut wrong_magic = protocol::header(&marker);
        wrong_magic[0] = 0;
        let mut trailing = protocol::header(&marker);
        trailing.push(0);

        for (case, bytes) in [
            ("version", wrong_version), // Reject an unsupported protocol version.
            ("magic", wrong_magic),     // Reject a different file signature.
            ("trailing", trailing),     // Reject bytes after the exact header.
            ("empty", Vec::new()),      // Reject an empty receipt.
        ] {
            fs::write(&path, bytes).unwrap();

            assert!(verified_stale_receipt(&path, &marker).is_err(), "{case}");
        }
    }
}
