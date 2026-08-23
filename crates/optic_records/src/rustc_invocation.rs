//! Records the Cargo-selected program chain that ultimately invokes rustc.
//!
//! Cargo can place an outer wrapper and a workspace wrapper in front of its selected compiler. A
//! single rustc path therefore does not describe the command that produced a build. An
//! [`RustcInvocation`] retains all three selections and yields them in execution order: outer
//! wrapper, workspace wrapper, then rustc.

use std::iter;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::validation::require_path;

/// The complete compiler program chain selected by Cargo.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedRustcInvocation")]
pub struct RustcInvocation {
    rustc: PathBuf,
    rustc_wrapper: Option<PathBuf>,
    rustc_workspace_wrapper: Option<PathBuf>,
}

impl RustcInvocation {
    /// Creates a compiler invocation from Cargo's three independent selections.
    ///
    /// # Errors
    ///
    /// Returns an error if any selected program path is empty.
    pub fn new(
        rustc: PathBuf,
        rustc_wrapper: Option<PathBuf>,
        rustc_workspace_wrapper: Option<PathBuf>,
    ) -> Result<Self, Error> {
        require_path("rustc program", &rustc)?;
        if let Some(wrapper) = &rustc_wrapper {
            require_path("rustc wrapper", wrapper)?;
        }
        if let Some(wrapper) = &rustc_workspace_wrapper {
            require_path("rustc workspace wrapper", wrapper)?;
        }

        Ok(Self {
            rustc,
            rustc_wrapper,
            rustc_workspace_wrapper,
        })
    }

    /// Returns the rustc executable selected by Cargo.
    #[must_use]
    pub fn rustc(&self) -> &Path {
        &self.rustc
    }

    /// Returns the outer wrapper applied to every rustc invocation.
    #[must_use]
    pub fn rustc_wrapper(&self) -> Option<&Path> {
        self.rustc_wrapper.as_deref()
    }

    /// Returns the additional wrapper applied to workspace members.
    #[must_use]
    pub fn rustc_workspace_wrapper(&self) -> Option<&Path> {
        self.rustc_workspace_wrapper.as_deref()
    }

    /// Yields the programs in the order Cargo executes them.
    pub fn programs(&self) -> impl Iterator<Item = &Path> {
        self.rustc_wrapper
            .iter()
            .chain(self.rustc_workspace_wrapper.iter())
            .map(PathBuf::as_path)
            .chain(iter::once(self.rustc.as_path()))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedRustcInvocation {
    rustc: PathBuf,
    rustc_wrapper: Option<PathBuf>,
    rustc_workspace_wrapper: Option<PathBuf>,
}

impl TryFrom<UncheckedRustcInvocation> for RustcInvocation {
    type Error = Error;

    fn try_from(invocation: UncheckedRustcInvocation) -> Result<Self, Self::Error> {
        Self::new(
            invocation.rustc,
            invocation.rustc_wrapper,
            invocation.rustc_workspace_wrapper,
        )
    }
}
