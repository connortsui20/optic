//! Captures and resolves Rust source from workspace and local path packages.
//!
//! Source bytes are copied before Cargo starts and validated after compilation. Query-time item
//! extraction parses only source blobs, never the current worktree.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use proc_macro2::Span;
use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use walkdir::{DirEntry, WalkDir};

use crate::{BuildSpec, Error, Result, SourceLocation, SourceView};

#[derive(Debug)]
pub(crate) struct SourceBaseline {
    /// Source snapshots that the store publishes with the capture.
    pub(crate) entries: Vec<SourceEntry>,

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

/// Source identity retained with a completed compiler run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingSourceBaseline {
    entries: Vec<PendingSourceEntry>,

    cache_inputs: Vec<PendingCacheInput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PendingSourceEntry {
    path: PathBuf,

    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PendingCacheInput {
    path: PathBuf,

    digest: String,
}

#[derive(Debug)]
pub(crate) struct StoredSource {
    /// The source path used by the captured build.
    pub(crate) path: String,

    /// The captured source bytes.
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct SourceItemRange {
    pub(crate) definition: Range<usize>,

    pub(crate) item: Range<usize>,

    pub(crate) start_line: usize,
}

impl SourceBaseline {
    pub(crate) fn capture(
        workspace_root: &Path,
        spec: &BuildSpec,
        staging_directory: &Path,
    ) -> Result<Self> {
        let source_directory = staging_directory.join("sources");
        fs::create_dir_all(&source_directory)
            .map_err(|source| Error::filesystem("create", &source_directory, source))?;
        let SourcePaths { paths, cache_paths } = source_paths(workspace_root, spec)?;

        let mut entries = Vec::with_capacity(paths.len());
        let mut source_digests = BTreeMap::new();
        for (index, path) in paths.iter().enumerate() {
            let snapshot = source_directory.join(format!("{index:08}.rs"));
            let digest = copy_and_hash(path, &snapshot)?;
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
                    hash_file(&path)?
                };

                Ok(CacheInput { path, digest })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            entries,
            cache_inputs,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for entry in &self.cache_inputs {
            let digest = match hash_file(&entry.path) {
                Ok(digest) => digest,
                Err(Error::Filesystem { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    return Err(Error::InputChanged {
                        path: entry.path.clone(),
                    });
                }
                Err(error) => return Err(error),
            };

            if digest != entry.digest {
                return Err(Error::InputChanged {
                    path: entry.path.clone(),
                });
            }
        }

        Ok(())
    }

    pub(crate) fn pending(&self) -> Result<PendingSourceBaseline> {
        let entries = self
            .entries
            .iter()
            .map(|entry| {
                Ok(PendingSourceEntry {
                    path: entry.path.clone(),
                    digest: hash_file(&entry.snapshot)?.to_hex().to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let cache_inputs = self
            .cache_inputs
            .iter()
            .map(|input| PendingCacheInput {
                path: input.path.clone(),
                digest: input.digest.to_hex().to_string(),
            })
            .collect();

        Ok(PendingSourceBaseline {
            entries,
            cache_inputs,
        })
    }

    pub(crate) fn resume(
        workspace_root: &Path,
        spec: &BuildSpec,
        staging_directory: &Path,
        pending: &PendingSourceBaseline,
        marker_path: &Path,
    ) -> Result<Self> {
        const MAX_SOURCE_ENTRIES: usize = 200_000;
        const MAX_CACHE_INPUTS: usize = 400_000;

        if pending.entries.len() > MAX_SOURCE_ENTRIES {
            return Err(Error::InvalidPendingEvidence {
                path: marker_path.to_owned(),
                message: format!(
                    "source entry count must be at most {MAX_SOURCE_ENTRIES}, got {}",
                    pending.entries.len()
                ),
            });
        }
        if pending.cache_inputs.len() > MAX_CACHE_INPUTS {
            return Err(Error::InvalidPendingEvidence {
                path: marker_path.to_owned(),
                message: format!(
                    "cache input count must be at most {MAX_CACHE_INPUTS}, got {}",
                    pending.cache_inputs.len()
                ),
            });
        }
        let expected = source_paths(workspace_root, spec)?;
        let entry_paths = pending
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let cache_paths = pending
            .cache_inputs
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        if entry_paths != expected.paths.into_iter().collect::<Vec<_>>()
            || cache_paths != expected.cache_paths.into_iter().collect::<Vec<_>>()
        {
            return Err(Error::PendingInputsChanged);
        }

        let source_directory = staging_directory.join("sources");
        let mut entries = Vec::with_capacity(pending.entries.len());

        for (index, entry) in pending.entries.iter().enumerate() {
            let digest = parse_digest(&entry.digest, marker_path)?;
            let snapshot = source_directory.join(format!("{index:08}.rs"));
            if hash_file(&snapshot)? != digest {
                return Err(Error::InvalidPendingEvidence {
                    path: marker_path.to_owned(),
                    message: format!("source snapshot digest does not match for index {index}"),
                });
            }
            entries.push(SourceEntry {
                path: entry.path.clone(),
                snapshot,
            });
        }

        let cache_inputs = pending
            .cache_inputs
            .iter()
            .map(|input| {
                Ok(CacheInput {
                    path: input.path.clone(),
                    digest: parse_digest(&input.digest, marker_path)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let baseline = Self {
            entries,
            cache_inputs,
        };
        baseline.validate()?;

        Ok(baseline)
    }
}

fn copy_and_hash(source: &Path, destination: &Path) -> Result<blake3::Hash> {
    let mut input = File::open(source).map_err(|error| Error::filesystem("open", source, error))?;
    let mut output = File::create(destination)
        .map_err(|error| Error::filesystem("create", destination, error))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let bytes = input
            .read(&mut buffer)
            .map_err(|error| Error::filesystem("read", source, error))?;
        if bytes == 0 {
            break;
        }

        hasher.update(&buffer[..bytes]);
        output
            .write_all(&buffer[..bytes])
            .map_err(|error| Error::filesystem("write", destination, error))?;
    }

    Ok(hasher.finalize())
}

fn hash_file(path: &Path) -> Result<blake3::Hash> {
    let mut file = File::open(path).map_err(|source| Error::filesystem("open", path, source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let bytes = file
            .read(&mut buffer)
            .map_err(|source| Error::filesystem("read", path, source))?;
        if bytes == 0 {
            break;
        }

        hasher.update(&buffer[..bytes]);
    }

    Ok(hasher.finalize())
}

struct SourcePaths {
    paths: BTreeSet<PathBuf>,
    cache_paths: BTreeSet<PathBuf>,
}

fn source_paths(workspace_root: &Path, spec: &BuildSpec) -> Result<SourcePaths> {
    let mut command = cargo_metadata::MetadataCommand::new();
    // NB: MetadataCommand cannot remove inherited variables. An empty value disables unstable
    // access for rustc probes that Cargo metadata can start.
    command
        .current_dir(workspace_root)
        .env("RUSTC_BOOTSTRAP", "");
    if let Some(path) = &spec.manifest_path {
        command.manifest_path(path);
    }
    if !spec.features.is_empty() {
        command.features(cargo_metadata::CargoOpt::SomeFeatures(
            spec.features.clone(),
        ));
    }
    if spec.all_features {
        command.features(cargo_metadata::CargoOpt::AllFeatures);
    }
    if spec.no_default_features {
        command.features(cargo_metadata::CargoOpt::NoDefaultFeatures);
    }
    command.other_options(metadata_options(spec));
    let metadata = command.exec()?;
    let local_packages = selected_local_packages(&metadata, spec);
    let mut paths = BTreeSet::new();
    let mut cache_paths = BTreeSet::new();

    for package in local_packages {
        cache_paths.insert(package.manifest_path.clone().into_std_path_buf());

        let Some(root) = package.manifest_path.parent() else {
            continue;
        };

        for entry in WalkDir::new(root).into_iter().filter_entry(included_entry) {
            let entry = entry
                .map_err(|source| Error::filesystem("walk", root.as_std_path(), source.into()))?;

            if entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "rs")
            {
                let path = fs::canonicalize(entry.path())
                    .map_err(|source| Error::filesystem("canonicalize", entry.path(), source))?;
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

    Ok(SourcePaths { paths, cache_paths })
}

fn parse_digest(digest: &str, marker_path: &Path) -> Result<blake3::Hash> {
    digest.parse().map_err(|_| Error::InvalidPendingEvidence {
        path: marker_path.to_owned(),
        message: format!(
            "BLAKE3 digest must contain 64 lowercase hexadecimal characters, got {digest}"
        ),
    })
}

fn selected_local_packages<'a>(
    metadata: &'a cargo_metadata::Metadata,
    spec: &BuildSpec,
) -> Vec<&'a cargo_metadata::Package> {
    let selected = spec
        .package
        .as_deref()
        .and_then(|name| {
            metadata
                .packages
                .iter()
                .find(|package| package.name == name)
        })
        .or_else(|| metadata.root_package());
    let Some(selected) = selected else {
        return metadata
            .packages
            .iter()
            .filter(|package| package.source.is_none())
            .collect();
    };
    let Some(resolve) = &metadata.resolve else {
        return vec![selected];
    };

    let nodes = resolve
        .nodes
        .iter()
        .map(|node| (&node.id, node))
        .collect::<HashMap<_, _>>();
    let mut reachable = HashSet::new();
    let mut pending = VecDeque::from([selected.id.clone()]);

    while let Some(package_id) = pending.pop_front() {
        if !reachable.insert(package_id.clone()) {
            continue;
        }
        let Some(node) = nodes.get(&package_id) else {
            continue;
        };

        pending.extend(node.dependencies.iter().cloned());
    }

    metadata
        .packages
        .iter()
        .filter(|package| package.source.is_none() && reachable.contains(&package.id))
        .collect()
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

pub(crate) fn find_item_at(location: &SourceLocation, source: &StoredSource) -> Option<SourceView> {
    let text = std::str::from_utf8(&source.bytes).ok()?;
    let file = syn::parse_file(text).ok()?;
    let byte_start = usize::try_from(location.byte_start).ok()?;
    let byte_end = usize::try_from(location.byte_end).ok()?;
    let mut visitor = ItemVisitor::default();
    visitor.visit_file(&file);
    let span = visitor
        .spans
        .into_iter()
        .find(|span| span.definition == (byte_start..byte_end))?
        .item;
    let start_line = span.start().line;
    let text = lines(text, start_line, span.end().line)?;

    Some(SourceView {
        path: source.path.clone(),
        start_line,
        text,
    })
}

pub(crate) fn source_item_ranges(path: &Path) -> Result<Vec<SourceItemRange>> {
    let bytes = fs::read(path).map_err(|source| Error::filesystem("read", path, source))?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(Vec::new());
    };
    let Ok(file) = syn::parse_file(text) else {
        return Ok(Vec::new());
    };
    let mut visitor = ItemVisitor::default();
    visitor.visit_file(&file);
    let line_starts = line_starts(text);
    let ranges = visitor
        .spans
        .into_iter()
        .filter_map(|span| {
            let start_line = span.item.start().line;
            let end_line = span.item.end().line;
            let start = *line_starts.get(start_line.checked_sub(1)?)?;
            let end = line_starts.get(end_line).copied().unwrap_or(text.len());

            Some(SourceItemRange {
                definition: span.definition,
                item: start..end,
                start_line,
            })
        })
        .collect();

    Ok(ranges)
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];

    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }

    starts
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

#[derive(Default)]
struct ItemVisitor {
    spans: Vec<FunctionSpan>,
}

struct FunctionSpan {
    definition: Range<usize>,
    item: Span,
}

impl<'ast> Visit<'ast> for ItemVisitor {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.spans
            .push(function_span(&function.vis, &function.sig, function.span()));
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.spans
            .push(function_span(&function.vis, &function.sig, function.span()));
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        let definition = function.sig.span().byte_range();
        self.spans.push(FunctionSpan {
            definition,
            item: function.span(),
        });
        visit::visit_trait_item_fn(self, function);
    }
}

fn function_span(
    visibility: &syn::Visibility,
    signature: &syn::Signature,
    item: Span,
) -> FunctionSpan {
    let signature = signature.span().byte_range();
    let start = match visibility {
        syn::Visibility::Inherited => signature.start,
        visibility => visibility.span().byte_range().start,
    };

    FunctionSpan {
        definition: start..signature.end,
        item,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        CacheInput, PendingSourceBaseline, PendingSourceEntry, SourceBaseline, StoredSource,
        find_item_at,
    };
    use crate::{BuildSpec, Error, SourceLocation};

    #[test]
    fn retained_baseline_reports_a_changed_input_as_stale() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let input = temporary.path().join("Cargo.toml");
        fs::write(&input, b"before").expect("the test can write the original input");
        let baseline = SourceBaseline {
            entries: Vec::new(),
            cache_inputs: vec![CacheInput {
                path: input.clone(),
                digest: blake3::hash(b"before"),
            }],
        };
        fs::write(&input, b"after").expect("the test can change the input");

        assert!(matches!(
            baseline.validate(),
            Err(Error::InputChanged { path }) if path == input
        ));
    }

    #[test]
    fn retained_paths_must_match_current_cargo_metadata_before_reads() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary workspace");
        fs::create_dir(temporary.path().join("src"))
            .expect("the test can create the source directory");
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname = \"pending-source\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        )
        .expect("the test can create a manifest");
        fs::write(temporary.path().join("src/lib.rs"), "pub fn value() {}\n")
            .expect("the test can create source");
        let pending = PendingSourceBaseline {
            entries: vec![PendingSourceEntry {
                path: temporary.path().join("not-a-cargo-input.rs"),
                digest: blake3::hash(b"").to_hex().to_string(),
            }],
            cache_inputs: Vec::new(),
        };

        let result = SourceBaseline::resume(
            temporary.path(),
            &BuildSpec::default(),
            &temporary.path().join("staging"),
            &pending,
            &temporary.path().join("pending.json"),
        );

        assert!(matches!(result, Err(Error::PendingInputsChanged)));
    }

    #[test]
    fn expands_an_exact_nested_span_to_the_complete_function() {
        let source = StoredSource {
            path: "src/kernel.rs".to_owned(),
            bytes: concat!(
                "fn outer() {\n",
                "    #[inline(always)]\n",
                "    fn chunk(value: u64) -> u64 {\n",
                "        value + 1\n",
                "    }\n",
                "    chunk(1);\n",
                "}\n",
            )
            .as_bytes()
            .to_vec(),
        };
        let location = SourceLocation {
            path: "src/kernel.rs".to_owned(),
            byte_start: 39,
            byte_end: 66,
            line_start: 3,
            column_start: 4,
            line_end: 3,
            column_end: 36,
        };

        let item =
            find_item_at(&location, &source).expect("the exact span selects the nested function");

        assert_eq!(item.start_line, 2);
        assert_eq!(
            item.text,
            "    #[inline(always)]\n    fn chunk(value: u64) -> u64 {\n        value + 1\n    }\n"
        );
    }

    #[test]
    fn exact_span_does_not_depend_on_parsing_the_definition_path() {
        let source = StoredSource {
            path: "src/kernel.rs".to_owned(),
            bytes: concat!(
                "struct Kernel<T>(T);\n",
                "\n",
                "impl<T> Kernel<T> {\n",
                "    fn new(value: T) -> Self {\n",
                "        Self(value)\n",
                "    }\n",
                "}\n",
            )
            .as_bytes()
            .to_vec(),
        };
        let location = SourceLocation {
            path: "src/kernel.rs".to_owned(),
            byte_start: 46,
            byte_end: 70,
            line_start: 4,
            column_start: 4,
            line_end: 4,
            column_end: 34,
        };

        let item = find_item_at(&location, &source)
            .expect("the compiler span selects the method without parsing its path");

        assert_eq!(item.start_line, 4);
        assert_eq!(
            item.text,
            "    fn new(value: T) -> Self {\n        Self(value)\n    }\n"
        );
    }

    #[test]
    fn does_not_return_an_enclosing_function_for_a_closure_span() {
        let source = StoredSource {
            path: "src/kernel.rs".to_owned(),
            bytes: b"fn kernel() {\n    let add = |left, right| left + right;\n}\n".to_vec(),
        };
        let location = SourceLocation {
            path: "src/kernel.rs".to_owned(),
            byte_start: 28,
            byte_end: 41,
            line_start: 2,
            column_start: 14,
            line_end: 2,
            column_end: 27,
        };

        assert!(find_item_at(&location, &source).is_none());
    }
}
