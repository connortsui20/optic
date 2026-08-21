//! Defines typed events for incremental application results.
//!
//! Streaming workflows call their consumer on the application thread. Each event contains bounded
//! metadata or one bounded text chunk. A future terminal client can consume these events without
//! parsing command output.

use serde::{Deserialize, Serialize};

use crate::{
    ArtifactSummary, CaptureId, CaptureMetadata, CompilerOutput, InstanceSummary, LlvmBodySummary,
    LlvmStage, RemarkFileSummary,
};

/// The maximum UTF-8 payload carried by one text event.
pub const TEXT_CHUNK_BYTES: usize = 64 * 1024;

/// The bounded result of a streamed row query.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamCount {
    /// The number of emitted rows.
    pub items: usize,
}

/// One event emitted while inspecting a capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum InspectEvent {
    /// Bounded capture metadata was resolved.
    Started {
        /// The request and compiler metadata.
        metadata: Box<CaptureMetadata>,
    },

    /// One captured LLVM artifact.
    Artifact {
        /// The artifact and its stage provenance.
        artifact: ArtifactSummary,
    },

    /// One raw optimization-remark file.
    RemarkFile {
        /// The remark file and its record count.
        remark_file: RemarkFileSummary,
    },
}

/// The bounded result of a streamed capture inspection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InspectSummary {
    /// The full capture identifier.
    pub capture_id: CaptureId,

    /// The number of emitted LLVM artifacts.
    pub artifacts: usize,

    /// The number of emitted optimization-remark files.
    pub remark_files: usize,
}

/// One high-level phase of an evidence capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapturePhase {
    /// Snapshots and validates local source inputs.
    Source,

    /// Runs Cargo and the selected rustc.
    Compile,

    /// Reads retained compiler artifacts into the staging catalog.
    Ingest,
}

/// One event emitted while capturing evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureEvent {
    /// A capture phase started.
    PhaseStarted(CapturePhase),

    /// A capture phase finished.
    PhaseFinished(CapturePhase),

    /// User-visible output arrived from Cargo or rustc.
    Cargo(cargo_ir::CargoProcessEvent),
}

/// One event emitted while showing compiler output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ShowEvent {
    /// The selected instance and capture were resolved.
    Started {
        /// The full capture identifier.
        capture_id: CaptureId,

        /// The selected concrete compiler instance.
        instance: InstanceSummary,

        /// The requested compiler output.
        output: CompilerOutput,
    },

    /// Captured Rust source is available.
    SourceStarted {
        /// The canonical captured source path.
        path: String,

        /// The one-based first displayed line.
        start_line: usize,
    },

    /// One bounded UTF-8 source chunk.
    SourceChunk {
        /// The source text.
        text: String,
    },

    /// The captured source item is complete.
    SourceFinished,

    /// One LLVM function body is about to be streamed.
    BodyStarted {
        /// The captured compiler stage.
        stage: LlvmStage,

        /// The compiler-owned module name.
        module: String,

        /// The raw LLVM symbol.
        symbol: String,
    },

    /// One bounded UTF-8 LLVM body chunk.
    BodyChunk {
        /// The LLVM text.
        text: String,
    },

    /// The current LLVM body is complete.
    BodyFinished {
        /// The incrementally computed body summary.
        summary: LlvmBodySummary,
    },
}

/// The bounded result of a streamed show operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShowSummary {
    /// The full capture identifier.
    pub capture_id: CaptureId,

    /// The selected concrete compiler instance.
    pub instance: InstanceSummary,

    /// The requested compiler output.
    pub output: CompilerOutput,

    /// The number of LLVM bodies emitted.
    pub bodies: usize,

    /// Whether a captured source item was emitted.
    pub source: bool,
}
