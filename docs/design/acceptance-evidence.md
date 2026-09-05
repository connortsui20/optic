# MVP acceptance evidence

This index maps the required claims to their owning tests. Test paths refer to the implementation
branch, not this docs-only branch. The [execution ledger](execution.md) records accepted revisions.
An existing test name is not proof that the integrated checkpoint has passed.

## Checkpoint A

| Claim | Owning test or file | Integrated status |
| --- | --- | --- |
| Canonical IDs and validated Cargo analysis records. | `cargo-optic-records/src/tests.rs` and identifier tests. | 31 tests pass locally. |
| Bounded durable reads and writes. | `cargo-optic-store/src/tests/bounds.rs`. | Passes locally. |
| Missing candidates differ from corruption. | `cargo-optic-store/src/tests/candidates.rs`. | Passes locally. |
| Publication has one final commit boundary. | `cargo-optic-store/src/tests/publication.rs`. | Passes locally. |
| Store initialization preserves user files. | `cargo-optic-store/src/tests/initialization.rs`. | Passes locally. |
| Fixtures build offline in private directories. | `cargo-optic-test-support/tests/workspace.rs`. | Passes locally. |
| Warm reuse preserves ID, time, and stored/build files. | `cargo-optic-api/tests/cache_journey.rs`. | Awaiting compiler integration. |
| Source, dependency, and build-script input edits invalidate. | `cargo-optic-api/tests/cache_journey.rs`. | Awaiting compiler integration. |
| A/B/A configuration changes cannot reuse old analysis. | `does_not_reuse_old_evidence_after_a_config_variant_returns`. | Awaiting compiler integration. |
| Git and non-Git default tracking ignore Optic output. | `excludes_store_output_from_default_build_script_tracking`. | Awaiting compiler integration. |
| Cargo profile fields remain authoritative. | `reuses_a_custom_profile_with_its_observed_settings`. | Awaiting compiler integration. |
| Cold, warm, stale, and forced requests do the expected compiler work. | `cargo-optic-compiler/tests/work_observation.rs`. | Awaiting compiler integration. |
| Driver cache avoids a second actual compilation. | Compiler unit-test process counter. | Awaiting compiler integration. |
| Compiler overrides fail and wrappers produce warnings. | `cargo-optic-compiler/tests/compiler_selection.rs`. | Awaiting compiler integration. |
| Failed compilation never publishes. | `failed_cargo_process_does_not_publish_a_capture`. | Awaiting compiler integration. |
| Failed publication cannot revive old freshness. | Additional cross-process API regression. | In progress. |
| Exact search, ordering, limits, and capture scope remain stable. | `cargo-optic-evidence/src/tests.rs` and API find tests. | Full integrated rerun pending. |
| Real Cargo discovers the CLI and distinguishes capture from reuse. | `cargo-optic/tests/e2e.rs`. | Awaiting compiler integration. |

The full store suite has 37 passing local tests. Records, store, and test-support Clippy checks pass.
The foundation-only CI revision passed Linux/macOS tests and failed the newly added standalone
driver formatting check. No Checkpoint A CI result exists yet.

Before accepting A, verify the remaining matrix dimensions from [test strategy](test-strategy.md).
In particular, record the tracked-environment and RUSTFLAGS regressions, Cargo-message rejection
cases, retry diagnostics, and missing-component diagnostic coverage.

## Later checkpoints

Checkpoint B has not started. Its evidence will cover reference resolution, normalized source
snapshots, proven optimized LLVM stages, bounded exact indexing, checked artifact reads, and the
complete cache journey with the final format.

Checkpoint C has not started. Its evidence will cover verified package archives, both installed CLI
hosts, the external library consumer, independent reviews, and the exact final CI revision.
