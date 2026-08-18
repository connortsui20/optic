//! Persistent compiler-evidence workflows for `cargo-optic`.
//!
//! [`Application`] connects the internal `cargo-ir` collector to a project-local evidence store.
//! The command-line binary is a thin renderer over these typed workflows. A future terminal client
//! can use the same application without parsing command output.

mod app;
pub use app::Application;

mod cli;
pub use cli::run_cli;

mod error;
pub use error::{Error, Result};

mod ids;
pub use ids::{CaptureId, InstanceId};

mod model;
pub use cargo_ir::LlvmStage;
pub use model::{
    BodyView, BuildSpec, CachePolicy, CaptureSummary, CleanSummary, CompilerOutput, FindResult,
    InstanceSummary, ShowView, SourceView,
};

mod source;

mod store;

mod terminal;
