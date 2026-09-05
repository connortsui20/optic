//! Searches the concrete compiler instances recorded for one capture.
//!
//! Queries read only the explicitly selected capture. The store crate owns durable evidence.

use optic_records::CaptureId;
use optic_records::InstanceRecord;
use optic_store::Store;
use snafu::ResultExt;

mod error;
pub use error::Error;

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
    instances: Vec<InstanceRecord>,
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
    pub fn instances(&self) -> &[InstanceRecord] {
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
        .filter(|instance| is_exact_match(instance, query))
        .collect::<Vec<_>>();
    let match_kind = if matches.is_empty() {
        matches.extend(
            instances
                .iter()
                .filter(|instance| is_substring_match(instance, query)),
        );

        MatchKind::Substring
    } else {
        MatchKind::Exact
    };

    matches.sort_by(|left, right| {
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
    let instances = matches.into_iter().take(limit).cloned().collect::<Vec<_>>();

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

#[cfg(test)]
mod tests;
