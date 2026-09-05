//! Identifies the immutable files declared by a capture.
//!
//! Artifact IDs are distinct from instance ordinals. File names derive from IDs alone, so durable
//! records never supply a path for the store to follow.

use serde::Deserialize;
use serde::Serialize;

/// A capture-local file identity, independent of instance order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ArtifactId(u64);

impl ArtifactId {
    /// Assigns a capture-local ID. The manifest rejects duplicate IDs.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric identity.
    pub fn value(self) -> u64 {
        self.0
    }
}

/// The evidence bytes contained in an artifact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Normalized UTF-8 source text loaded by the compiler.
    Source,
    /// Unchanged textual LLVM from the matching disassembler.
    Llvm,
}

/// One generated file and its exact encoded length in bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRecord {
    id: ArtifactId,
    kind: ArtifactKind,
    byte_len: u64,
}

impl ArtifactRecord {
    /// Declares an artifact. Empty artifacts are valid.
    pub fn new(id: ArtifactId, kind: ArtifactKind, byte_len: u64) -> Self {
        Self { id, kind, byte_len }
    }

    /// Returns the capture-local artifact identity.
    pub fn id(&self) -> ArtifactId {
        self.id
    }

    /// Returns the evidence format of the file.
    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    /// Returns the exact stored length in bytes.
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Returns `artifact-` followed by exactly 16 lowercase hexadecimal digits.
    pub fn file_name(&self) -> String {
        format!("artifact-{:016x}", self.id.0)
    }
}
