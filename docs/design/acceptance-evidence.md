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

## Checkpoint B

| Claim | Owning test or file | Integrated status |
| --- | --- | --- |
| References preserve durable ordinals through sorting and limiting. | `cargo-optic-evidence/src/tests/find.rs` and `references.rs`. | Passes locally. |
| Source and LLVM availability remain distinct from corrupt storage. | Evidence `tests/source.rs` and `tests/llvm.rs`. | Passes locally. |
| Exact symbols, module-local aliases, cycles, and multiple bodies have deterministic results. | Evidence `tests/llvm.rs`. | Passes locally. |
| Artifact publication preserves the final commit boundary. | Store `tests/artifacts.rs` and `tests/publication.rs`. | Passes locally. |
| Finite ranges support sparse offsets above 4 GiB and report caller-writer errors. | Store `tests/artifacts.rs`. | Passes locally. |
| LLVM indexing bounds retained headers and streams unrelated input. | Compiler `src/llvm_index/` tests. | 20 worker tests pass; compiler integration pending. |
| Normalized whole source, exact LLVM, old snapshots, and unsupported configurations reach callers. | API `tests/show.rs` and CLI `tests/e2e.rs`. | Added; runtime verification pending. |
| Full-format evidence survives the complete cache journey. | API `tests/cache_journey.rs`. | Added; runtime verification pending. |

All 109 records, store, and evidence tests pass together locally. The compiler-stage proof and full
integration remain required before accepting this checkpoint.

## Checkpoint C

Checkpoint C has not started. Its evidence will cover verified package archives, both installed CLI
hosts, the external library consumer, independent reviews, and the exact final CI revision.
