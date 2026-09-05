//! Owns selected-target evidence and its temporary files through store publication.
//!
//! Collection materializes and indexes complete optimized modules before returning this owner.
//! The capture layer supplies an ID, validates the durable manifest, then retains the returned
//! directory until the store has copied its declared artifacts.

use optic_records::ArtifactRecord;
use optic_records::BuildRecord;
use optic_records::CaptureAnalysis;
use optic_records::CaptureId;
use optic_records::CompilerIdentity;
use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::LlvmCollection;
use optic_records::LlvmProvenance;
use tempfile::TempDir;

use crate::Error;

/// One successful Cargo build and the compiler evidence collected from its selected target.
pub struct CollectedBuild {
    build: BuildRecord,
    compiler: CompilerIdentity,
    instances: Vec<InstanceRecord>,
    analysis: CaptureAnalysis,
    artifacts: Vec<ArtifactRecord>,
    provenance: LlvmProvenance,
    llvm: LlvmCollection,
    temporary: TempDir,
}

impl CollectedBuild {
    pub(crate) fn new(
        build: BuildRecord,
        compiler: CompilerIdentity,
        evidence: crate::artifacts::CollectedEvidence,
        analysis: CaptureAnalysis,
        temporary: TempDir,
    ) -> Self {
        Self {
            build,
            compiler,
            instances: evidence.instances,
            analysis,
            artifacts: evidence.artifacts,
            provenance: evidence.provenance,
            llvm: evidence.llvm,
            temporary,
        }
    }

    /// Validates the capture-scoped manifest and transfers its temporary artifact directory.
    ///
    /// The caller must retain the directory until store publication finishes.
    ///
    /// # Errors
    ///
    /// Returns an error if the complete evidence violates the durable manifest contract.
    pub fn into_parts(
        self,
        capture_id: CaptureId,
    ) -> Result<
        (
            BuildRecord,
            CompilerIdentity,
            CaptureAnalysis,
            InstanceManifest,
            TempDir,
        ),
        Error,
    > {
        let manifest = InstanceManifest::new(
            capture_id,
            self.instances,
            self.artifacts,
            self.provenance,
            self.llvm,
        )?;

        Ok((
            self.build,
            self.compiler,
            self.analysis,
            manifest,
            self.temporary,
        ))
    }
}
