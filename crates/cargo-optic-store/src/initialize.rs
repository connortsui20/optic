//! Prepares ignored Optic state before Cargo evaluates package freshness.
//!
//! Git-backed default package scans need the local ignore file before compilation. Non-Git scans
//! already exclude dot-prefixed paths. Explicit package inclusion or build-script tracking of
//! `.optic` can still invalidate capture freshness.

use std::fs;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
use std::path::Path;

use snafu::IntoError;
use snafu::ResultExt;

use crate::Error;
use crate::Store;
use crate::error::ConflictingIgnoreFileSnafu;
use crate::error::FilesystemSnafu;
use crate::error::UnexpectedFileTypeSnafu;

impl Store {
    /// Creates ignored store state before the first Cargo build or freshness probe.
    ///
    /// An existing `.optic/.gitignore` containing exactly `*\n` needs no write. Explicit Cargo
    /// inclusion or build-script tracking of `.optic` remains unsupported for capture reuse.
    ///
    /// # Errors
    ///
    /// Returns an error for conflicting ignore contents, symlinks, wrong file types, or failed I/O.
    /// Different existing ignore contents remain unchanged.
    pub fn initialize(&self) -> Result<(), Error> {
        let optic = self
            .root
            .parent()
            .expect("Store::new places the store beneath .optic");
        create_directory(optic)?;
        let ignore = optic.join(".gitignore");

        match fs::symlink_metadata(&ignore) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    return UnexpectedFileTypeSnafu {
                        expected: "file",
                        path: ignore,
                    }
                    .fail();
                }

                if metadata.len() != 2 {
                    return ConflictingIgnoreFileSnafu { path: ignore }.fail();
                }

                let mut bytes = Vec::new();
                fs::File::open(&ignore)
                    .and_then(|file| file.take(3).read_to_end(&mut bytes))
                    .with_context(|_| FilesystemSnafu {
                        operation: "read",
                        path: ignore.clone(),
                    })?;

                if bytes != b"*\n" {
                    return ConflictingIgnoreFileSnafu { path: ignore }.fail();
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&ignore)
                    .with_context(|_| FilesystemSnafu {
                        operation: "create",
                        path: ignore.clone(),
                    })?;

                file.write_all(b"*\n").with_context(|_| FilesystemSnafu {
                    operation: "write",
                    path: ignore,
                })?;
            }
            Err(source) => {
                return Err(FilesystemSnafu {
                    operation: "read metadata for",
                    path: ignore,
                }
                .into_error(source));
            }
        }

        create_directory(&self.root)?;

        for name in ["staging", "captures", "candidates"] {
            create_directory(&self.root.join(name))?;
        }

        Ok(())
    }

    /// Validates every owned ancestor before a read can interpret an absent entry as a miss.
    pub(crate) fn namespace_exists(&self, namespace: &str) -> Result<bool, Error> {
        let optic = self
            .root
            .parent()
            .expect("Store::new places the store beneath .optic");

        for path in [
            optic.to_owned(),
            self.root.clone(),
            self.root.join(namespace),
        ] {
            if !directory_exists(&path)? {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

fn create_directory(path: &Path) -> Result<(), Error> {
    if !directory_exists(path)? {
        fs::create_dir(path).with_context(|_| FilesystemSnafu {
            operation: "create",
            path: path.to_owned(),
        })?;
    }

    Ok(())
}

fn directory_exists(path: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => UnexpectedFileTypeSnafu {
            expected: "directory",
            path: path.to_owned(),
        }
        .fail(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(FilesystemSnafu {
            operation: "read metadata for",
            path: path.to_owned(),
        }
        .into_error(source)),
    }
}
