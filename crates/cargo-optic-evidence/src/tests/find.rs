//! Checks literal search precedence, result order, and immutable references.
//!
//! Search order differs from manifest order. Every result must retain its original reference scope.

use optic_records::InstanceRef;

use super::TestStore;
use super::instance;
use crate::Error;
use crate::MatchKind;
use crate::find_instances;

#[test]
fn exact_names_take_precedence_over_substrings() {
    let fixture = TestStore::new();
    let capture_id = fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![
            // Exact definition-path match.
            instance(
                "fixture",
                "kernel",
                "fixture::definition_match",
                "definition_symbol",
            ), //
            // Exact display-name match.
            instance(
                "fixture",
                "fixture::display_match",
                "kernel",
                "display_symbol",
            ), //
            // Exact raw-symbol match.
            instance(
                "fixture",
                "fixture::symbol_match",
                "fixture::symbol_match::<u64>",
                "kernel",
            ), //
            // Substring-only match.
            instance(
                "fixture",
                "fixture::other",
                "fixture::other::<kernel>",
                "substring_symbol",
            ), //
        ],
    );

    let found = find_instances(&fixture.store, &capture_id, "kernel", 10)
        .expect("the exact instance can be found");
    let raw_symbols = found
        .instances()
        .iter()
        .map(|instance| instance.record().raw_symbol())
        .collect::<Vec<_>>();

    assert_eq!(found.capture_id(), &capture_id);
    assert_eq!(found.match_kind(), MatchKind::Exact);
    assert_eq!(found.total_matches(), 3);
    assert!(!found.is_truncated());
    assert_eq!(
        raw_symbols,
        vec!["definition_symbol", "kernel", "display_symbol"]
    );
}

#[test]
fn substring_search_is_case_sensitive() {
    let fixture = TestStore::new();
    let capture_id = fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![instance(
            "fixture",
            "fixture::Kernel",
            "fixture::Kernel::<u64>",
            "_Kernel_u64",
        )],
    );

    let found = find_instances(&fixture.store, &capture_id, "kernel", 10)
        .expect("the case-sensitive search can complete");

    assert_eq!(found.match_kind(), MatchKind::Substring);
    assert_eq!(found.total_matches(), 0);
    assert!(found.instances().is_empty());
    assert!(!found.is_truncated());
}

#[test]
fn stable_order_is_applied_before_the_result_limit() {
    let fixture = TestStore::new();
    let capture_id = fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![
            // Fifth sorted result, omitted by the limit.
            instance("b_crate", "b::needle", "b-needle", "symbol_b"), //
            // First sorted result.
            instance("z_crate", "z::needle", "a-needle", "symbol_e"), //
            // Fourth sorted result.
            instance("b_crate", "b::needle", "b-needle", "symbol_a"), //
            // Second sorted result.
            instance("z_crate", "a::needle", "b-needle", "symbol_d"), //
            // Third sorted result.
            instance("a_crate", "b::needle", "b-needle", "symbol_c"), //
        ],
    );

    let found = find_instances(&fixture.store, &capture_id, "needle", 4)
        .expect("the bounded search can complete");
    let raw_symbols = found
        .instances()
        .iter()
        .map(|instance| instance.record().raw_symbol())
        .collect::<Vec<_>>();

    assert_eq!(found.match_kind(), MatchKind::Substring);
    assert_eq!(found.total_matches(), 5);
    assert!(found.is_truncated());
    assert_eq!(
        raw_symbols,
        vec!["symbol_e", "symbol_d", "symbol_c", "symbol_a"]
    );

    let ordinals = found
        .instances()
        .iter()
        .map(|instance| instance.reference().ordinal())
        .collect::<Vec<_>>();

    assert_eq!(ordinals, vec![1, 3, 4, 2]);

    let manifest = fixture.store.read_instances(&capture_id).unwrap();

    for found in found.instances() {
        assert_eq!(found.reference().capture_id(), &capture_id);
        assert_eq!(
            found.record(),
            &manifest.instances()[found.reference().ordinal() as usize]
        );
    }
}

#[test]
fn result_set_is_scoped_to_the_selected_capture() {
    let fixture = TestStore::new();
    let selected_id = fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![instance(
            "selected",
            "selected::kernel",
            "selected::kernel",
            "selected_symbol",
        )],
    );
    fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzx",
        vec![instance(
            "other",
            "other::kernel",
            "other::kernel",
            "other_symbol",
        )],
    );

    let found = find_instances(&fixture.store, &selected_id, "kernel", 10)
        .expect("the selected capture can be searched");

    assert_eq!(found.capture_id(), &selected_id);
    assert_eq!(found.instances().len(), 1);
    assert_eq!(
        found.instances()[0].record().raw_symbol(),
        "selected_symbol"
    );
    assert_eq!(
        found.instances()[0].reference(),
        &InstanceRef::new(selected_id, 0)
    );
}

#[test]
fn rejects_an_empty_query() {
    let fixture = TestStore::new();
    let capture_id = fixture.publish("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", Vec::new());

    let error = find_instances(&fixture.store, &capture_id, "", 1)
        .expect_err("an empty query must be rejected before reading the store");

    assert!(matches!(error, Error::EmptyQuery { query } if query.is_empty()));
}

#[test]
fn rejects_a_zero_result_limit() {
    let fixture = TestStore::new();
    let capture_id = fixture.publish("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy", Vec::new());

    let zero_limit = find_instances(&fixture.store, &capture_id, "kernel", 0)
        .expect_err("a zero limit must be rejected before reading the store");

    assert!(matches!(zero_limit, Error::InvalidLimit { actual: 0 }));
}

#[test]
fn equal_display_names_keep_distinct_references() {
    let fixture = TestStore::new();
    let id = fixture.publish(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        vec![
            instance("fixture", "second", "same", "symbol"), //
            instance("fixture", "first", "same", "symbol"),  //
        ],
    );

    let found = find_instances(&fixture.store, &id, "same", 2).unwrap();

    assert_eq!(found.match_kind(), MatchKind::Exact);
    assert_eq!(found.total_matches(), 2);
    assert_eq!(
        found.instances()[0].reference(),
        &InstanceRef::new(id.clone(), 1)
    );
    assert_eq!(found.instances()[1].reference(), &InstanceRef::new(id, 0));
}
