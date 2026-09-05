//! Protects capture-scoped instance search.
//!
//! Tests publish durable records before checking precedence, ordering, bounds, and isolation.

use std::path::PathBuf;

use optic_records::BuildRecord;
use optic_records::CaptureId;
use optic_records::CaptureRecord;
use optic_records::CargoTargetKind;
use optic_records::CompilerIdentity;
use optic_records::DefinitionRecord;
use optic_records::InstanceManifest;
use optic_records::InstanceRecord;
use optic_records::PlacementRecord;
use optic_records::TargetRecord;
use optic_store::Store;

use crate::Error;
use crate::MatchKind;
use crate::find_instances;

struct TestStore {
    /// Keeps the temporary workspace alive for the lifetime of the store handle.
    temporary: tempfile::TempDir,
    /// The store under test.
    store: Store,
}

impl TestStore {
    fn new() -> Self {
        let temporary = tempfile::tempdir().expect("the test workspace can be created");
        let store = Store::new(temporary.path()).expect("the fixture store path is valid");

        Self { temporary, store }
    }

    fn publish(&self, id: &str, instances: Vec<InstanceRecord>) -> CaptureId {
        let id = id
            .parse::<CaptureId>()
            .expect("the fixture capture ID is valid");
        let target = TargetRecord::new("fixture", CargoTargetKind::Lib)
            .expect("the fixture target is valid");
        let build = BuildRecord::new(
            "fixture",
            "0.1.0",
            target,
            "release",
            PathBuf::from("cargo"),
            self.temporary.path().to_owned(),
            vec!["rustc".to_owned()],
        )
        .expect("the fixture build is valid");
        let sysroot = self.temporary.path().join("toolchain");
        let compiler = CompilerIdentity::new(
            sysroot
                .join("bin")
                .join(format!("rustc{}", std::env::consts::EXE_SUFFIX)),
            "1.99.0-nightly",
            "0123456789abcdef0123456789abcdef01234567",
            "x86_64-unknown-linux-gnu",
            sysroot,
        )
        .expect("the fixture compiler identity is valid");
        let manifest = InstanceManifest::new(id.clone(), instances)
            .expect("the fixture instance manifest is valid");
        let capture = CaptureRecord::new(id.clone(), 1_000, build, compiler);

        self.store
            .publish(&capture, &manifest)
            .expect("the fixture capture can be published");

        id
    }
}

fn instance(
    crate_name: &str,
    definition_path: &str,
    display_name: &str,
    raw_symbol: &str,
) -> InstanceRecord {
    let definition = DefinitionRecord::new(crate_name, definition_path)
        .expect("the fixture definition is valid");
    let placement = PlacementRecord::new("fixture.0", "external", "default", false, 1)
        .expect("the fixture placement is valid");

    InstanceRecord::new(definition, display_name, raw_symbol, vec![placement])
        .expect("the fixture instance is valid")
}

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
        .map(InstanceRecord::raw_symbol)
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
            // Fourth sorted result.
            instance("b_crate", "b::needle", "b-needle", "symbol_b"), //
            // First sorted result.
            instance("z_crate", "z::needle", "a-needle", "symbol_e"), //
            // Fifth sorted result, omitted by the limit.
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
        .map(InstanceRecord::raw_symbol)
        .collect::<Vec<_>>();

    assert_eq!(found.match_kind(), MatchKind::Substring);
    assert_eq!(found.total_matches(), 5);
    assert!(found.is_truncated());
    assert_eq!(
        raw_symbols,
        vec!["symbol_e", "symbol_d", "symbol_c", "symbol_a"]
    );
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
    assert_eq!(found.instances()[0].raw_symbol(), "selected_symbol");
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
