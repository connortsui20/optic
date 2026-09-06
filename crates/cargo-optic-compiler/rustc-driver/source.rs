//! Captures whole local definition spans from rustc's normalized source map.
//!
//! Current files supply canonical path checks only. Snapshot bytes always come from the compiler's
//! loaded source, so BOM and CRLF normalization agree with rustc's byte positions.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use rustc_hir::Node;
use rustc_middle::ty::Instance;
use rustc_middle::ty::InstanceKind;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;

use crate::manifest::ManifestWriter;
use crate::protocol;

pub(crate) enum Source {
    /// A checked whole-item range in a compiler-loaded snapshot.
    Available(SourceSpan),
    /// Uses one of the unavailable source codes defined by the shared private protocol.
    Unavailable(u32),
}

pub(crate) struct SourceSpan {
    /// The generated source-file ID, shared by all definitions in this file.
    pub(crate) artifact: u64,
    /// The byte offset in normalized UTF-8 source text.
    pub(crate) start: u64,
    /// The whole-item length in normalized UTF-8 bytes.
    pub(crate) length: u64,
    /// The canonical source path, used for display only after collection.
    pub(crate) display_path: PathBuf,
    /// The one-based line containing the first byte.
    pub(crate) starting_line: u64,
}

pub(crate) struct Snapshots {
    directory: PathBuf,
    package_root: PathBuf,
    /// Maps canonical paths to the source-map position and generated artifact ID already written.
    files: BTreeMap<PathBuf, (u32, u64)>,
}

impl Snapshots {
    pub(crate) fn new(directory: PathBuf, package_root: PathBuf) -> Self {
        Self {
            directory,
            package_root,
            files: BTreeMap::new(),
        }
    }

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

        let artifact = match self.files.get(&path) {
            Some(&(position, id)) => {
                if position != file.start_pos.0 {
                    return Err(io::Error::other(
                        "one source path must identify one compiler-loaded file",
                    ));
                }
                id
            }
            None => {
                let id = u64::try_from(self.files.len()).map_err(io::Error::other)?;
                fs::write(
                    self.directory.join(format!("artifact-{id:016x}")),
                    text.as_bytes(),
                )?;
                manifest.write_source_file(id, text.len() as u64)?;
                self.files.insert(path.clone(), (file.start_pos.0, id));
                id
            }
        };

        Ok(Source::Available(SourceSpan {
            artifact,
            start: start as u64,
            length: (end - start) as u64,
            display_path: path,
            starting_line: 1 + line as u64,
        }))
    }
}
