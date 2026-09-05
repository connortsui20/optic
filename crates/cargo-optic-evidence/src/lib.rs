//! Searches concrete compiler instances and selects their stored source or LLVM evidence.
//!
//! Queries read only the explicitly selected capture. [`find_instances`] supplies immutable
//! references for [`source_evidence`] and [`llvm_evidence`]. The store validates durable records and
//! artifact files, then copies the selected ranges through [`Store::copy_evidence`].

use optic_records::CaptureId;
use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::InstanceRef;
use optic_store::Store;
use snafu::ResultExt;

mod error;
pub use error::Error;

mod evidence_range;
pub use evidence_range::EvidenceRange;

mod source;
pub use source::SourceEvidence;
pub use source::source_evidence;

mod llvm;
pub use llvm::LlvmBody;
pub use llvm::LlvmEvidence;
pub use llvm::llvm_evidence;

/// A search result with its immutable position in the selected capture.
///
/// Equal display names remain distinct instances. The reference belongs only to the exact capture
/// and durable format that produced the record.
#[derive(Clone, Debug)]
pub struct FoundInstance {
    reference: InstanceRef,
    record: InstanceRecord,
}

impl FoundInstance {
    /// Returns the reference for the instance's original manifest position.
    pub fn reference(&self) -> &InstanceRef {
        &self.reference
    }

    /// Returns the stored instance and its searchable names.
    pub fn record(&self) -> &InstanceRecord {
        &self.record
    }
}

/// How a query matched its returned concrete compiler instances.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchKind {
    /// Every returned instance exactly matched one of its searchable identities.
    Exact,
    /// Every returned instance contained the query in one of its searchable identities.
    Substring,
}

/// The complete facts about one capture-scoped instance search.
///
/// The returned instances can be limited, but [`FindResults::total_matches`] always reports the
/// number of instances that matched before applying the limit.
#[derive(Clone, Debug)]
pub struct FindResults {
    capture_id: CaptureId,
    match_kind: MatchKind,
    total_matches: usize,
    instances: Vec<FoundInstance>,
}

impl FindResults {
    /// Returns the capture that scopes these results.
    pub fn capture_id(&self) -> &CaptureId {
        &self.capture_id
    }

    /// Returns how the query matched these results.
    pub fn match_kind(&self) -> MatchKind {
        self.match_kind
    }

    /// Returns the number of matches before the result limit was applied.
    pub fn total_matches(&self) -> usize {
        self.total_matches
    }

    /// Returns the concrete compiler instances retained by the result limit.
    pub fn instances(&self) -> &[FoundInstance] {
        &self.instances
    }

    /// Returns whether the result limit omitted one or more matching instances.
    pub fn is_truncated(&self) -> bool {
        self.total_matches > self.instances.len()
    }
}

/// Finds concrete compiler instances within `capture_id`.
///
/// Exact definition paths, display names, and raw symbols take precedence over all substring
/// matches. Substring matching is case-sensitive and literal. Results are sorted independently of
/// their durable order by display name, definition path, crate name, and raw symbol before `limit`
/// is applied. A valid query with no match returns an empty substring result set.
///
/// # Errors
///
/// Returns an error if `query` is empty, `limit` is zero, or the selected capture's instance
/// manifest cannot be read.
pub fn find_instances(
    store: &Store,
    capture_id: &CaptureId,
    query: &str,
    limit: usize,
) -> Result<FindResults, Error> {
    if query.is_empty() {
        return error::EmptyQuerySnafu {
            query: query.to_owned(),
        }
        .fail();
    }

    if limit == 0 {
        return error::InvalidLimitSnafu { actual: limit }.fail();
    }

    let manifest = store
        .read_instances(capture_id)
        .context(error::StoreSnafu)?;
    let instances = manifest.instances();

    let mut matches = instances
        .iter()
        // The ordinal identifies the instance. Enumerating after filtering or sorting changes it.
        .enumerate()
        .filter(|(_, instance)| is_exact_match(instance, query))
        .collect::<Vec<_>>();
    let match_kind = if matches.is_empty() {
        matches.extend(
            instances
                .iter()
                .enumerate()
                .filter(|(_, instance)| is_substring_match(instance, query)),
        );

        MatchKind::Substring
    } else {
        MatchKind::Exact
    };

    matches.sort_by(|(_, left), (_, right)| {
        left.display_name()
            .cmp(right.display_name())
            .then_with(|| {
                left.definition()
                    .definition_path()
                    .cmp(right.definition().definition_path())
            })
            .then_with(|| {
                left.definition()
                    .crate_name()
                    .cmp(right.definition().crate_name())
            })
            .then_with(|| left.raw_symbol().cmp(right.raw_symbol()))
    });
    let total_matches = matches.len();
    let instances = matches
        .into_iter()
        .take(limit)
        .map(|(ordinal, record)| FoundInstance {
            reference: InstanceRef::new(
                capture_id.clone(),
                u64::try_from(ordinal)
                    .expect("manifest positions fit in u64 on supported platforms"),
            ),
            record: record.clone(),
        })
        .collect::<Vec<_>>();

    Ok(FindResults {
        capture_id: capture_id.clone(),
        match_kind,
        total_matches,
        instances,
    })
}

fn is_exact_match(instance: &InstanceRecord, query: &str) -> bool {
    instance.definition().definition_path() == query
        || instance.display_name() == query
        || instance.raw_symbol() == query
}

fn is_substring_match(instance: &InstanceRecord, query: &str) -> bool {
    instance.definition().definition_path().contains(query)
        || instance.display_name().contains(query)
        || instance.raw_symbol().contains(query)
}

fn referenced_instance<'a>(
    manifest: &'a InstanceManifest,
    reference: &InstanceRef,
) -> Result<&'a InstanceRecord, Error> {
    let instance = usize::try_from(reference.ordinal())
        .ok()
        .and_then(|ordinal| manifest.instances().get(ordinal));

    instance.ok_or_else(|| {
        error::InvalidInstanceReferenceSnafu {
            reference: reference.clone(),
            instance_count: manifest.instances().len(),
        }
        .build()
    })
}

#[cfg(test)]
mod tests;
