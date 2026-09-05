# MVP execution ledger

Implementation starts from `main` at `684991a` plus the CI commit `b18b658` on `ct/complete-mvp`.
The approved specification is the plan at `15a4ccb`. Draft PR #17 carries the implementation.
Checkpoint A is accepted at `0813fba`. Checkpoints B and C remain incomplete.

## Ownership

- The integration owner owns capture orchestration, API, CLI, root manifests, CI, and documentation.
- The compiler worker owns compiler product files and compiler unit tests in `/tmp/optic-mvp-compiler`.
- The storage worker owns records, store, and their unit tests in `/tmp/optic-mvp-store`.
- The testing worker owns test support and existing `tests/` integration files in `/tmp/optic-mvp-tests`.
- During B, the evidence worker owns the evidence crate in `/tmp/optic-mvp-evidence`.
- During B, the indexer worker owns only compiler `src/llvm_index/` in `/tmp/optic-mvp-llvm-index`.
- Workers leave root manifests and the lockfile to the integration owner.

Workers use separate branches and return commits with their test results. Only the integration owner
publishes the MVP PR or merges to main. Source/LLVM implementation waits for Checkpoint A.

## Checkpoint A interfaces

Records define `CaptureKey` as a validated SHA-256 hexadecimal digest and `AnalysisToken` as a fresh
UUID-v4 hexadecimal token. Distinct types prevent swapping a request identity with one compilation.

`CargoArtifactRecord` wraps Cargo metadata's existing `Artifact` value. Its constructor normalizes
unordered fields and clears `fresh` before storage/comparison. Validation requires complete target
identity and absolute source/output paths. Reuse compares this stored observation, not guessed
profile settings. Using the existing Cargo type avoids another manually maintained artifact schema.
Records use this dependency only for data, never to invoke Cargo.

`CaptureAnalysis` contains the request key, token, and normalized artifact observation.
`CaptureRecord::new` accepts this required analysis record after build and compiler provenance.
The capture format revision changes with this schema. No compatibility constructor is required.

The compiler exposes `prepare_build(workspace, request) -> PreparedBuild` and these methods:

- `PreparedBuild::request_key()` returns the normalized request key.
- `PreparedBuild::probe(&mut self, &CaptureAnalysis)` returns `Freshness::{Fresh, Stale}` or an error.
- `PreparedBuild::collect(self)` returns one `CollectedBuild` with a newly generated analysis token.
- `CollectedBuild::into_parts()` returns build, compiler, instances, and analysis records.

The store exposes `initialize()` before any build/probe. `read_candidate(&CaptureKey)` returns an
optional validated capture record. `publish(capture, instances)` installs the pointer immediately
before its existing final directory rename. The returned candidate includes the analysis needed by
the compiler probe. Candidate validation reads the instance manifest before returning it.

The integration owner adds `CapturePolicy::{Reuse, Fresh}` and
`CaptureOutcome::{Captured(CaptureRecord), Reused(CaptureRecord)}` in the capture crate.
`Optic::capture(request, policy)` exposes these semantics. Existing compiler convenience entry points
can remain during test migration, but remove unused compatibility wrappers before final review.

The prepared operation owns a stopped probe's diagnostic file until collection finishes. A mutable
probe and consuming collection keep that lifetime explicit without shared or interior-mutable
state. Collection failure replays the retained probe diagnostics. Collection success discards the
intentional probe abort.

Use `sha2` 0.10.9 for driver/request digests and the existing Cargo metadata library for structured
messages. Keep the existing private driver protocol and extend its documented records as required.

## Status

- [x] Create the integration branch with the existing CI commit.
- [x] Allocate isolated compiler, storage, and testing worktrees.
- [x] Commit the pinned toolchain, standalone formatting check, CI timeouts, and contributor guide.
- [x] Establish reproducible checks and the shared test harness.
- [x] Implement and verify Checkpoint A on Linux and macOS.
- [ ] Implement and verify narrow show with the final cache format.
- [ ] Verify packaged installation and external library use.
- [ ] Resolve independent correctness and Rust-style reviews.
- [ ] Merge the final verified MVP and reconcile planning status.

Update this ledger with implementation findings before dependent work starts. The detailed cache,
show, testing, and installation documents remain the owning behavior contracts.
The [acceptance evidence index](acceptance-evidence.md) maps claims to the owning tests.
The [show interfaces](show-interfaces.md) fix the next checkpoint's worker boundaries before code.

## Current verification

Rust 1.98.1 and its required components are installed locally. The accepted revision `0813fba`
passes 149 workspace tests, standalone/workspace formatting, Clippy with warnings denied, and
rustdoc with warnings denied. Draft PR #17 contains this revision. All jobs in
[CI run 33996076997](https://github.com/connortsui20/optic/actions/runs/33996076997) pass, including
Linux and macOS workspace tests. Checkpoint A is complete.

The cache tests cover real selected-target and driver work counts, cold/warm/stale/forced requests,
source/dependency/build-script changes, tracked environment, RUSTFLAGS, and A/B/A configuration
transitions. A publication-failure test executes the newly compiled binary, proves the old capture
remains searchable, and requires a new capture after restoring the old pointer.

The first integrated run exposed an overly strict test assertion. Cargo can rewrite its `.d`
bookkeeping file on a fresh invocation. The corrected test preserves full stored-file metadata,
target/build inventories and sizes, and the selected executable's modification time. Independent
positive work counts still establish that warm reuse performs no compiler analysis.

Real example/benchmark selection and invocation-subdirectory coverage pass in the final A revision.
B starts from this accepted revision under [the persisted interfaces](show-interfaces.md). Before C
packaging, remove the compiler unit process tests' dependency on the unpublished helper from retained
package tests.

## Checkpoint B progress

All B worktrees start from accepted A revision `0813fba`. Their shared signatures are fixed in
[show interfaces](show-interfaces.md). The compiler worker owns the indexer's module declaration and
caller, but does not edit its implementation directory.

The integration owner committed API/CLI and capture-lifetime wiring as `b5fc405`. The records handoff
is integrated as `f8b8d88`, with durable format 4, required source availability, capture-scoped
references, generated artifacts, checked ranges, module-owned definitions, and LLVM provenance.
All 39 records tests and focused Clippy pass locally.

The compiler worker is verifying the pinned compiler's optimized artifact stages before enabling
LLVM availability. Store artifact I/O, exact indexing, evidence queries, and process acceptance are
in progress. These partial commits are not a validated B implementation and have not replaced the
accepted A revision on the PR.
