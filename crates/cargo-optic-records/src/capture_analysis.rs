//! Associates a capture with the Cargo invocation that produced its evidence.
//!
//! The request key locates a candidate. Its token and artifact observation let the compiler ask
//! Cargo to validate freshness without compiling that token again.

use serde::Deserialize;
use serde::Serialize;

use crate::AnalysisToken;
use crate::CaptureKey;
use crate::CargoArtifactRecord;

/// The required analysis identity and selected Cargo artifact for one capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureAnalysis {
    request_key: CaptureKey,
    token: AnalysisToken,
    artifact: CargoArtifactRecord,
}

impl CaptureAnalysis {
    /// Associates validated request, compilation, and artifact identities.
    pub fn new(
        request_key: CaptureKey,
        token: AnalysisToken,
        artifact: CargoArtifactRecord,
    ) -> Self {
        Self {
            request_key,
            token,
            artifact,
        }
    }

    /// Returns the normalized request identity used to locate this candidate.
    pub fn request_key(&self) -> &CaptureKey {
        &self.request_key
    }

    /// Returns the token that only freshness probes can reuse after publication.
    pub fn token(&self) -> &AnalysisToken {
        &self.token
    }

    /// Returns the normalized selected-target artifact observation.
    pub fn artifact(&self) -> &CargoArtifactRecord {
        &self.artifact
    }
}
