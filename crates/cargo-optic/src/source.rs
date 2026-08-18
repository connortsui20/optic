//! Captures and resolves Rust source from workspace and local path packages.
//!
//! Source bytes are copied before Cargo starts and validated after compilation. Query-time item
//! extraction parses only source blobs, never the current worktree.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use walkdir::{DirEntry, WalkDir};

use crate::{BuildSpec, Error, Result, SourceView};

#[derive(Debug)]
pub(crate) struct SourceBaseline {
    /// Source snapshots that the store publishes with the capture.
    pub(crate) entries: Vec<SourceEntry>,

    /// A digest of Cargo metadata before compilation.
    metadata_digest: blake3::Hash,

    /// Files that must remain unchanged until compilation finishes.
    cache_inputs: Vec<CacheInput>,
}

#[derive(Debug)]
pub(crate) struct SourceEntry {
    /// The source path used by Cargo.
    pub(crate) path: PathBuf,

    /// The immutable copy made before compilation.
    pub(crate) snapshot: PathBuf,
}

#[derive(Debug)]
struct CacheInput {
    path: PathBuf,
    digest: blake3::Hash,
}

#[derive(Debug)]
pub(crate) struct StoredSource {
    /// The source path used by the captured build.
    pub(crate) path: String,

    /// The captured source bytes.
    pub(crate) bytes: Vec<u8>,
}

impl SourceBaseline {
    pub(crate) fn capture(
        workspace_root: &Path,
        spec: &BuildSpec,
        staging_directory: &Path,
    ) -> Result<Self> {
        let mut command = cargo_metadata::MetadataCommand::new();
        command.current_dir(workspace_root);
        if let Some(path) = &spec.manifest_path {
            command.manifest_path(path);
        }
        command.other_options(metadata_options(spec));
        let metadata = command.exec()?;
        let metadata_digest = blake3::hash(&serde_json::to_vec(&metadata)?);
        let source_directory = staging_directory.join("sources");
        fs::create_dir_all(&source_directory)
            .map_err(|source| Error::filesystem("create", &source_directory, source))?;

        let local_packages = metadata
            .packages
            .iter()
            .filter(|package| package.source.is_none())
            .collect::<Vec<_>>();
        let mut paths = BTreeSet::new();
        let mut cache_paths = BTreeSet::new();

        for package in local_packages {
            cache_paths.insert(package.manifest_path.clone().into_std_path_buf());

            let Some(root) = package.manifest_path.parent() else {
                continue;
            };

            for entry in WalkDir::new(root).into_iter().filter_entry(included_entry) {
                let entry = entry.map_err(|source| {
                    Error::filesystem("walk", root.as_std_path(), source.into())
                })?;

                if entry.file_type().is_file()
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "rs")
                {
                    let path = entry.into_path();
                    paths.insert(path.clone());
                    cache_paths.insert(path);
                }
            }
        }

        let lock_file = workspace_root.join("Cargo.lock");
        if lock_file.is_file() {
            cache_paths.insert(lock_file);
        }
        for path in cargo_configuration_paths(workspace_root) {
            cache_paths.insert(path);
        }

        let mut entries = Vec::with_capacity(paths.len());
        let mut source_digests = BTreeMap::new();
        for (index, path) in paths.iter().enumerate() {
            let bytes = fs::read(path).map_err(|source| Error::filesystem("read", path, source))?;
            let digest = blake3::hash(&bytes);
            let snapshot = source_directory.join(format!("{index:08}.rs"));
            fs::write(&snapshot, bytes)
                .map_err(|source| Error::filesystem("write", &snapshot, source))?;
            entries.push(SourceEntry {
                path: path.clone(),
                snapshot,
            });
            source_digests.insert(path.clone(), digest);
        }

        let cache_inputs = cache_paths
            .into_iter()
            .map(|path| {
                let digest = if let Some(digest) = source_digests.get(&path) {
                    *digest
                } else {
                    let bytes = fs::read(&path)
                        .map_err(|source| Error::filesystem("read", &path, source))?;

                    blake3::hash(&bytes)
                };

                Ok(CacheInput { path, digest })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            entries,
            metadata_digest,
            cache_inputs,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for entry in &self.cache_inputs {
            let bytes = fs::read(&entry.path)
                .map_err(|source| Error::filesystem("read", &entry.path, source))?;

            if blake3::hash(&bytes) != entry.digest {
                return Err(Error::InputChanged {
                    path: entry.path.clone(),
                });
            }
        }

        Ok(())
    }

    pub(crate) fn cache_digest(&self) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.metadata_digest.as_bytes());

        for entry in &self.cache_inputs {
            let path = entry.path.to_string_lossy();
            let path_length = u64::try_from(path.len())
                .expect("usize path lengths fit in u64 on supported Rust targets");
            hasher.update(&path_length.to_le_bytes());
            hasher.update(path.as_bytes());
            hasher.update(entry.digest.as_bytes());
        }

        hasher.finalize()
    }
}

fn metadata_options(spec: &BuildSpec) -> Vec<String> {
    let mut options = Vec::new();

    if spec.locked {
        options.push("--locked".to_owned());
    }
    if spec.offline {
        options.push("--offline".to_owned());
    }
    if spec.frozen {
        options.push("--frozen".to_owned());
    }

    options
}

fn cargo_configuration_paths(workspace_root: &Path) -> Vec<PathBuf> {
    let mut paths = workspace_root
        .ancestors()
        .flat_map(|directory| {
            [
                directory.join(".cargo/config"),
                directory.join(".cargo/config.toml"),
            ]
        })
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));

    if let Some(cargo_home) = cargo_home {
        for name in ["config", "config.toml"] {
            let path = cargo_home.join(name);

            if path.is_file() {
                paths.push(path);
            }
        }
    }

    paths.sort();
    paths.dedup();

    paths
}

pub(crate) fn find_item(definition: &str, sources: &[StoredSource]) -> Option<SourceView> {
    let name = function_name(definition)?;
    let mut candidates = Vec::new();

    for source in sources {
        let Ok(text) = std::str::from_utf8(&source.bytes) else {
            continue;
        };
        let Ok(file) = syn::parse_file(text) else {
            continue;
        };
        let mut visitor = ItemVisitor {
            name,
            spans: Vec::new(),
        };
        visitor.visit_file(&file);

        for span in visitor.spans {
            let start = span.start().line;
            let end = span.end().line;
            let Some(item) = lines(text, start, end) else {
                continue;
            };
            let score = path_score(definition, &source.path);
            candidates.push((score, source.path.clone(), start, item));
        }
    }

    candidates.sort_by_key(|candidate| Reverse(candidate.0));
    let best = candidates.first()?;

    if candidates.get(1).is_some_and(|next| next.0 == best.0) {
        return None;
    }

    Some(SourceView {
        path: best.1.clone(),
        start_line: best.2,
        text: best.3.clone(),
    })
}

fn included_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    !matches!(
        entry.file_name().to_str(),
        Some(".git" | ".optic" | "target")
    )
}

fn function_name(definition: &str) -> Option<&str> {
    let definition = definition.split("::<").next().unwrap_or(definition);
    definition
        .rsplit("::")
        .next()
        .filter(|name| !name.is_empty())
}

fn path_score(definition: &str, path: &str) -> usize {
    definition
        .split("::")
        .filter(|segment| !segment.is_empty())
        .filter(|segment| path.contains(segment))
        .count()
}

fn lines(text: &str, start: usize, end: usize) -> Option<String> {
    if start == 0 || end < start {
        return None;
    }

    let lines: Vec<_> = text.lines().collect();
    let selected = lines.get(start - 1..end)?;
    let mut result = selected.join("\n");
    result.push('\n');

    Some(result)
}

struct ItemVisitor<'a> {
    name: &'a str,
    spans: Vec<Span>,
}

impl<'ast> Visit<'ast> for ItemVisitor<'_> {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if function.sig.ident == self.name {
            self.spans.push(function.span());
        }
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        if function.sig.ident == self.name {
            self.spans.push(function.span());
        }
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        if function.sig.ident == self.name {
            self.spans.push(function.span());
        }
        visit::visit_trait_item_fn(self, function);
    }
}

#[cfg(test)]
mod tests {
    use super::{StoredSource, find_item};

    #[test]
    fn extracts_a_complete_function_when_another_source_is_invalid() {
        let sources = vec![
            StoredSource {
                path: "src/invalid.rs".to_owned(),
                bytes: b"this is not Rust".to_vec(),
            },
            StoredSource {
                path: "src/kernel.rs".to_owned(),
                bytes: b"fn other() {}\n\npub fn kernel<T>(value: T) {\n    drop(value);\n}\n"
                    .to_vec(),
            },
        ];

        let source = find_item("crate::kernel", &sources).expect("the fixture contains one kernel");

        assert_eq!(source.path, "src/kernel.rs");
        assert_eq!(source.start_line, 3);
        assert_eq!(
            source.text,
            "pub fn kernel<T>(value: T) {\n    drop(value);\n}\n"
        );
    }
}
