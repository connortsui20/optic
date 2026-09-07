//! Captures whole local definition spans from rustc's normalized source map.
//!
//! Current files supply canonical path checks only. Snapshot bytes always come from the compiler's
//! loaded source, so BOM and CRLF normalization agree with rustc's byte positions.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use rustc_hir::Node;
use rustc_middle::ty::Instance;
use rustc_middle::ty::InstanceKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;

use crate::manifest::ManifestWriter;
use crate::protocol;

/// A captured source range or the protocol reason that prevents whole-item attribution.
pub(crate) enum Source {
    /// A checked whole-item range in a compiler-loaded snapshot.
    Available(SourceSpan),
    /// Uses one of the unavailable source codes defined by the shared private protocol.
    Unavailable(u32),
}

/// A whole-item range validated against one normalized compiler-loaded file.
///
/// [`Snapshots::capture`] checks the range and line number before construction. The byte range can
/// be empty, but it **must** remain within the snapshot and on UTF-8 boundaries.
pub(crate) struct SourceSpan {
    /// The attempt-local source-file ID, shared by definitions in this snapshot. See [`protocol`].
    pub(crate) artifact: u64,

    /// The byte offset in normalized UTF-8 source text.
    pub(crate) start: u64,

    /// The whole-item length in normalized UTF-8 bytes.
    pub(crate) length: u64,

    /// The canonical source path, used for display only after collection.
    pub(crate) display_path: PathBuf,

    /// The one-based line containing the starting position, including an empty range.
    pub(crate) starting_line: u64,
}

/// Deduplicates normalized source snapshots within one selected-target collection attempt.
pub(crate) struct Snapshots {
    /// The private attempt directory supplied by the collector.
    directory: PathBuf,
    /// The canonical package root used to reject source outside the selected package.
    package_root: PathBuf,
    /// Maps canonical source paths to files whose bytes and manifest records are already written.
    files: BTreeMap<PathBuf, CapturedFile>,
}

/// Identifies the compiler file that supplied an already-written snapshot.
struct CapturedFile {
    /// Rustc's source-map start, retained to detect distinct files with one canonical path.
    source_map_start: u32,
    /// The attempt-local ID used by the snapshot file and its manifest declaration.
    artifact: u64,
}

impl Snapshots {
    /// Shares source snapshots across instances in one collection attempt.
    ///
    /// The directory **must** be the private attempt directory. The package root **must** be
    /// canonical so [`Self::capture`] can compare canonical source paths against it.
    pub(crate) fn new(directory: PathBuf, package_root: PathBuf) -> Self {
        Self {
            directory,
            package_root,
            files: BTreeMap::new(),
        }
    }

    /// Captures a whole local item from one compiler-loaded file inside the package root.
    ///
    /// The range uses normalized UTF-8 bytes. Each canonical path is written and declared in the
    /// manifest once. Unsupported definitions return [`Source::Unavailable`]. Returns an error if a
    /// snapshot or manifest write fails, or one path identifies conflicting compiler files.
    pub(crate) fn capture<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        instance: Instance<'tcx>,
        manifest: &mut ManifestWriter,
    ) -> io::Result<Source> {
        let Some(local) = instance.def_id().as_local() else {
            return Ok(Source::Unavailable(protocol::SOURCE_NONLOCAL));
        };

        if !matches!(instance.def, InstanceKind::Item(_)) {
            return Ok(Source::Unavailable(protocol::SOURCE_GENERATED));
        }

        let hir_id = tcx.local_def_id_to_hir_id(local);

        if matches!(tcx.hir_node(hir_id), Node::Synthetic) {
            return Ok(Source::Unavailable(protocol::SOURCE_GENERATED));
        }

        // `def_span` can end at the signature. HIR's whole-item span includes the function body.
        let span = tcx.hir_span_with_body(hir_id);

        if span.from_expansion() {
            return Ok(Source::Unavailable(protocol::SOURCE_GENERATED));
        }

        if span.is_dummy() || span.hi() < span.lo() {
            return Ok(Source::Unavailable(protocol::SOURCE_UNSUPPORTED_SPAN));
        }

        let source_map = tcx.sess.source_map();
        let file = source_map.lookup_source_file(span.lo());
        let last = source_map.lookup_source_file(span.hi());

        if file.start_pos != last.start_pos {
            return Ok(Source::Unavailable(protocol::SOURCE_UNSUPPORTED_SPAN));
        }

        let Some(text) = &file.src else {
            return Ok(Source::Unavailable(protocol::SOURCE_UNLOADED));
        };

        let FileName::Real(name) = &file.name else {
            return Ok(Source::Unavailable(protocol::SOURCE_GENERATED));
        };

        let Some(path) = name.local_path() else {
            return Ok(Source::Unavailable(protocol::SOURCE_UNLOADED));
        };

        let Ok(path) = fs::canonicalize(path) else {
            return Ok(Source::Unavailable(protocol::SOURCE_UNSUPPORTED_SPAN));
        };

        if !path.starts_with(&self.package_root) {
            return Ok(Source::Unavailable(protocol::SOURCE_OUTSIDE_PACKAGE));
        }

        let start = (span.lo() - file.start_pos).0 as usize;
        let end = (span.hi() - file.start_pos).0 as usize;

        if text.get(start..end).is_none() {
            return Ok(Source::Unavailable(protocol::SOURCE_UNSUPPORTED_SPAN));
        }

        // Rustc's line index uses normalized byte positions and returns a zero-based line.
        let Some(line) = file.lookup_line(file.relative_position(span.lo())) else {
            return Ok(Source::Unavailable(protocol::SOURCE_UNSUPPORTED_SPAN));
        };

        let artifact = self.capture_file(&path, file.start_pos.0, text, manifest)?;

        Ok(Source::Available(SourceSpan {
            artifact,
            start: start as u64,
            length: (end - start) as u64,
            display_path: path,
            starting_line: 1 + line as u64,
        }))
    }

    /// Reuses a checked file identity or writes its first snapshot and manifest declaration.
    ///
    /// The caller supplies the canonical path, source-map start, and normalized text from one
    /// compiler file. A reused path **must** identify that same file.
    fn capture_file(
        &mut self,
        path: &Path,
        source_map_start: u32,
        text: &str,
        manifest: &mut ManifestWriter,
    ) -> io::Result<u64> {
        if let Some(file) = self.files.get(path) {
            if file.source_map_start != source_map_start {
                return Err(io::Error::other(
                    "one source path must identify one compiler-loaded file",
                ));
            }

            return Ok(file.artifact);
        }

        let artifact = u64::try_from(self.files.len()).map_err(io::Error::other)?;
        fs::write(
            self.directory.join(format!("artifact-{artifact:016x}")),
            text.as_bytes(),
        )?;
        manifest.write_source_file(artifact, text.len() as u64)?;
        self.files.insert(
            path.to_owned(),
            CapturedFile {
                source_map_start,
                artifact,
            },
        );

        Ok(artifact)
    }
}
