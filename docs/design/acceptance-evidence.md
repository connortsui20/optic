# MVP acceptance evidence

This index maps the required claims to their owning tests. Test paths refer to the implementation
branch, not this docs-only branch. The [execution ledger](execution.md) records accepted revisions.
An existing test name is not proof that the integrated checkpoint has passed.

## Checkpoint A

| Claim | Owning test or file | Integrated status |
| --- | --- | --- |
| Canonical IDs and validated Cargo analysis records. | `cargo-optic-records/src/tests/` and identifier tests. | Passes in accepted A. |
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
| Exact search, ordering, limits, and capture scope remain stable. | `cargo-optic-evidence/src/tests/` and API find tests. | Passes in accepted A. |
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
| LLVM indexing bounds retained headers and streams unrelated input. | Compiler `src/llvm_index/` tests. | Passes in accepted B. |
| Normalized whole source, exact LLVM, old snapshots, and unsupported configurations reach callers. | API `tests/show.rs` and CLI `tests/e2e.rs`. | Passes in accepted B. |
| Full-format evidence survives the complete cache journey. | API `tests/cache_journey.rs`. | Passes in accepted B. |

The accepted B revision is `ed164a3`. All 226 workspace tests and quality checks pass locally.
[CI run 33997850037](https://github.com/connortsui20/optic/actions/runs/33997850037) passes quality,
Linux tests, and macOS tests on that revision. Every B row above passes in the integrated suite.
The stage proof runs both supported modes with the pinned compiler and matching LLVM 22.1.8.

## Checkpoint C

| Claim | Owning test or file | Integrated status |
| --- | --- | --- |
| Seven product archives pass Cargo verification with no unpublished helper dependency. | `scripts/check-install.sh`. | Passes locally at `d2ebd20`. |
| Unpacked compiler unit tests need only extracted product dependencies. | Installation script and compiler unit tests. | All 54 pass at `d2ebd20`. |
| Installed Cargo discovery, capture/list/find, source/LLVM, changed source, and forced analysis work. | Installation script's CLI journey. | Passes locally at `d2ebd20`. |
| A separate consumer needs only the public `optic` API. | `scripts/install-fixtures/consumer/`. | Build, journey, Clippy, and rustdoc pass at `d2ebd20`. |
| The installed tool captures its own committed source and reuses that evidence. | Installation script's Linux self-hosting journey. | `CaptureId::generate` source, LLVM, and warm reuse pass at `d2ebd20`. |
| Missing placement modules are corruption, not unavailable evidence or a reusable capture. | Records `tests/evidence.rs` and store `tests/artifacts.rs`. | Passes locally. |
| Parser boundary tests force actual short reads. | Compiler `src/llvm_index/tests.rs` and its consumers. | All 21 indexer tests pass locally. |
| Named protocol codes preserve their wire values and meanings. | Compiler `src/manifest/tests.rs`. | Passes locally. |
| Both independent reviews resolve their findings. | Execution ledger. | Both approve `514b8a0`. |
| The exact merge candidate passes both-host workspace and installed-product CI. | [CI run 33999328890](https://github.com/connortsui20/optic/actions/runs/33999328890). | All jobs pass at `514b8a0`. |

The final candidate `514b8a0` passes all 236 workspace tests, formatting, Clippy with warnings denied,
and public/private rustdoc with warnings denied. Its complete installation repeat passes with evidence
at `/tmp/optic-install.I30jpiuX`, including every installed-product row above. Final both-host CI passes.

The first complete installed run retained its evidence at `/tmp/optic-install.0Tnsjkqs` locally.
It used Rust 1.98.1, compiler commit `48a229ceaefd4985c50990b14116b6d856af0985`, and LLVM 22.1.8.
