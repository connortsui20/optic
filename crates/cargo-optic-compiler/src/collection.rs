//! Owns validated selected-target instances and their Cargo analysis identity.
//!
//! Collection loads the complete private manifest before its temporary directory is removed.
//! Publication belongs to the capture and store crates.

use optic_records::BuildRecord;
use optic_records::CaptureAnalysis;
use optic_records::CompilerIdentity;
use optic_records::InstanceRecord;

use crate::BuildRequest;
use crate::Error;
use crate::Workspace;

/// One successful Cargo build and the compiler evidence collected from its selected target.
pub struct CollectedBuild {
    build: BuildRecord,
    compiler: CompilerIdentity,
    instances: Vec<InstanceRecord>,
    analysis: CaptureAnalysis,
}

impl CollectedBuild {
    pub(crate) fn new(
        build: BuildRecord,
        compiler: CompilerIdentity,
        instances: Vec<InstanceRecord>,
        analysis: CaptureAnalysis,
    ) -> Self {
        Self {
            build,
            compiler,
            instances,
            analysis,
        }
    }

    /// Separates build provenance, compiler identity, concrete instances, and analysis identity.
    pub fn into_parts(
        self,
    ) -> (
        BuildRecord,
        CompilerIdentity,
        Vec<InstanceRecord>,
        CaptureAnalysis,
    ) {
        (self.build, self.compiler, self.instances, self.analysis)
    }
}

/// Prepares and collects an explicit target with a newly generated analysis token.
///
/// # Errors
///
/// Returns the preparation or collection errors documented by [`crate::prepare_build`] and
/// [`crate::PreparedBuild::collect`].
pub fn collect_build(
    workspace: &Workspace,
    request: &BuildRequest,
) -> Result<CollectedBuild, Error> {
    crate::prepare_build(workspace, request)?.collect()
}
