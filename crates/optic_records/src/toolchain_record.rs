//! Records the stable identity reported by the compiler used for a capture.
//!
//! [`ToolchainRecord`] stores the selected rustc path and the identity fields from `rustc -vV`.
//! The compiler crate resolves wrappers and Cargo configuration before constructing this value, so
//! this module owns field validity but not compiler selection policy.

use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::validation::require_path;
use crate::validation::require_text;

/// Stable identity fields reported by the rustc used for a capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedToolchainRecord")]
pub struct ToolchainRecord {
    rustc: PathBuf,
    release: String,
    commit_hash: String,
    host: String,
    llvm_version: String,
}

impl ToolchainRecord {
    /// Creates an identity from the selected rustc and its verbose version output.
    ///
    /// # Errors
    ///
    /// Returns an error if the program path or any identity field is empty.
    pub fn new(
        rustc: PathBuf,
        release: impl Into<String>,
        commit_hash: impl Into<String>,
        host: impl Into<String>,
        llvm_version: impl Into<String>,
    ) -> Result<Self, Error> {
        let release = release.into();
        let commit_hash = commit_hash.into();
        let host = host.into();
        let llvm_version = llvm_version.into();

        require_path("rustc program", &rustc)?;
        require_text("rustc release", &release)?;
        require_text("rustc commit hash", &commit_hash)?;
        require_text("rustc host", &host)?;
        require_text("LLVM version", &llvm_version)?;

        Ok(Self {
            rustc,
            release,
            commit_hash,
            host,
            llvm_version,
        })
    }

    /// Returns the rustc executable selected by Cargo configuration.
    #[must_use]
    pub fn rustc(&self) -> &Path {
        &self.rustc
    }

    /// Returns the `release` field from `rustc -vV`.
    #[must_use]
    pub fn release(&self) -> &str {
        &self.release
    }

    /// Returns the `commit-hash` field from `rustc -vV`.
    #[must_use]
    pub fn commit_hash(&self) -> &str {
        &self.commit_hash
    }

    /// Returns the `host` field from `rustc -vV`.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the `LLVM version` field from `rustc -vV`.
    #[must_use]
    pub fn llvm_version(&self) -> &str {
        &self.llvm_version
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedToolchainRecord {
    rustc: PathBuf,
    release: String,
    commit_hash: String,
    host: String,
    llvm_version: String,
}

impl TryFrom<UncheckedToolchainRecord> for ToolchainRecord {
    type Error = Error;

    fn try_from(record: UncheckedToolchainRecord) -> Result<Self, Self::Error> {
        Self::new(
            record.rustc,
            record.release,
            record.commit_hash,
            record.host,
            record.llvm_version,
        )
    }
}
