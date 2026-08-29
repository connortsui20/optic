//! Identifies the compiler that produced one capture.
//!
//! [`CompilerIdentity`] records the selected rustc executable and the identity fields reported by
//! that compiler. The compiler boundary supplies canonical paths, but this record validates only
//! their absolute, lexically normalized representation. It does not consult the filesystem again.

use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::validation::require_absolute_normalized_path;
use crate::validation::require_text;

/// The exact compiler used for the selected target invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RawCompilerIdentity")]
pub struct CompilerIdentity {
    rustc: PathBuf,
    release: String,
    commit_hash: String,
    host: String,
    sysroot: PathBuf,
}

impl CompilerIdentity {
    /// Creates an identity from values reported by the selected compiler.
    ///
    /// # Errors
    ///
    /// Returns an error if a textual value is empty or either path is not absolute and lexically
    /// normalized.
    pub fn new(
        rustc: PathBuf,
        release: impl Into<String>,
        commit_hash: impl Into<String>,
        host: impl Into<String>,
        sysroot: PathBuf,
    ) -> Result<Self, Error> {
        let release = release.into();
        let commit_hash = commit_hash.into();
        let host = host.into();

        require_absolute_normalized_path("rustc path", &rustc)?;
        require_text("rustc release", &release)?;
        require_text("rustc commit hash", &commit_hash)?;
        require_text("rustc host", &host)?;
        require_absolute_normalized_path("rustc sysroot", &sysroot)?;

        Ok(Self {
            rustc,
            release,
            commit_hash,
            host,
            sysroot,
        })
    }

    /// Returns the absolute, lexically normalized rustc executable path.
    pub fn rustc(&self) -> &Path {
        &self.rustc
    }

    /// Returns rustc's complete release string.
    pub fn release(&self) -> &str {
        &self.release
    }

    /// Returns rustc's full commit hash as reported by `rustc -vV`.
    pub fn commit_hash(&self) -> &str {
        &self.commit_hash
    }

    /// Returns rustc's host triple.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns rustc's absolute, lexically normalized sysroot path.
    pub fn sysroot(&self) -> &Path {
        &self.sysroot
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompilerIdentity {
    rustc: PathBuf,
    release: String,
    commit_hash: String,
    host: String,
    sysroot: PathBuf,
}

impl TryFrom<RawCompilerIdentity> for CompilerIdentity {
    type Error = Error;

    fn try_from(identity: RawCompilerIdentity) -> Result<Self, Self::Error> {
        Self::new(
            identity.rustc,
            identity.release,
            identity.commit_hash,
            identity.host,
            identity.sysroot,
        )
    }
}
