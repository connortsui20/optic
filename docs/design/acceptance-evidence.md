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
| Warm reuse preserves ID, time, and stored/build files. | `cargo-optic-api/tests/cache_journey.rs`. | Passes in accepted A. |
| Source, dependency, and build-script input edits invalidate. | `cargo-optic-api/tests/cache_journey.rs`. | Passes in accepted A. |
| A/B/A configuration changes cannot reuse old analysis. | `does_not_reuse_old_evidence_after_a_config_variant_returns`. | Passes in accepted A. |
| Git and non-Git default tracking ignore Optic output. | `excludes_store_output_from_default_build_script_tracking`. | Passes in accepted A. |
| Cargo profile fields remain authoritative. | `reuses_a_custom_profile_with_its_observed_settings`. | Passes in accepted A. |
| Cold, warm, stale, and forced requests do the expected compiler work. | `cargo-optic-compiler/tests/work_observation.rs`. | Passes in accepted A. |
| Driver cache avoids a second actual compilation. | Compiler unit-test process counter. | Passes in accepted A. |
| Compiler overrides fail and wrappers produce warnings. | `cargo-optic-compiler/tests/compiler_selection.rs`. | Passes in accepted A. |
| Failed compilation never publishes. | `failed_cargo_process_does_not_publish_a_capture`. | Passes in accepted A. |
| Failed publication cannot revive old freshness. | `rejects_old_evidence_after_collection_succeeds_but_publication_fails`. | Passes in accepted A. |
| Exact search, ordering, limits, and capture scope remain stable. | `cargo-optic-evidence/src/tests.rs` and API find tests. | Passes in accepted A. |
| Real Cargo discovers the CLI and distinguishes capture from reuse. | `cargo-optic/tests/e2e.rs`. | Passes in accepted A. |

The accepted A revision is `0813fba`. All 149 workspace tests and quality checks pass locally.
[CI run 33996076997](https://github.com/connortsui20/optic/actions/runs/33996076997) passes quality,
Linux tests, and macOS tests on that revision.

The final matrix additions are `invalidates_build_script_environment_and_rustflags`,
`collects_and_probes_named_targets_from_a_subdirectory`, and the compiler unit tests for Cargo
observations, stopped-probe receipts, retry diagnostics, and failed driver-build guidance. The latter
uses an isolated failing compiler shim, without changing installed components.

## Later checkpoints

Checkpoint B follows the accepted A revision. Its evidence will cover reference resolution, normalized source
snapshots, proven optimized LLVM stages, bounded exact indexing, checked artifact reads, and the
complete cache journey with the final format.

Checkpoint C has not started. Its evidence will cover verified package archives, both installed CLI
hosts, the external library consumer, independent reviews, and the exact final CI revision.
