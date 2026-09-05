# MVP execution ledger

Implementation starts from `main` at `684991a` plus the CI commit `b18b658` on `ct/complete-mvp`.
The approved specification is the plan at `15a4ccb`. Draft PR #17 carries the implementation.
Checkpoint A is accepted at `0813fba`. Checkpoint B is accepted at `ed164a3`. C remains incomplete.

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
- [x] Implement and verify narrow show with the final cache format.
- [x] Verify packaged installation and external library use.
- [x] Resolve independent correctness and Rust-style reviews.
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

Store artifact I/O is integrated as `1b557c1`. Candidate reads return the validated manifest, and
publication copies only declared generated artifact names before the existing commit boundary.
Finite reads use checked 64-bit ranges and distinguish caller-writer errors from stored-file errors.
Evidence queries are integrated as `0e3fe1f`, with search allocation simplified in `54303b2`.
All 109 combined records, store, and evidence tests pass locally on the integration branch.

The bounded LLVM indexer is integrated as `0cb5f8f`. Its worker passed 20 focused tests, an LLVM
22.1.8 assembly round trip, and a Rust-generated module check. Compiler wiring passed in accepted B.
API/CLI process acceptance adds whole functions and methods, shared generic source, Unicode/BOM/CRLF,
unsupported provenance, both optimized stages, stored bytes after edits, and closed stdout.
These tests pass against the integrated compiler in accepted B.

The compiler worker verified both optimized stages on the pinned compiler. The proof found four
regular modules per mode and preserved effective codegen configuration. The
[stage contract](show.md#selecting-the-artifact-stage) records the exact source and test evidence.
Compiler handoffs `d98f141`, `76a0308`, and `6c74758` pass 65 compiler tests and quality checks in
the worker. They are ready for integration and full process acceptance.

The integrated B revision `ed164a3` passes all 226 workspace tests, formatting, Clippy, and rustdoc
locally. PR #17 contains this revision. All jobs in
[CI run 33997850037](https://github.com/connortsui20/optic/actions/runs/33997850037) pass, including
Linux and macOS workspace tests. Checkpoint B is accepted, and C can start.

## Checkpoint C handoff

After B acceptance, the integration owner updates the seven product packages to `0.1.0`, adds
release metadata, owns root manifests/lockfile and CI, and finishes the product documentation.
An installation worker owns only `scripts/check-install.sh` and its copied consumer/fixture assets.
The script packages with verification enabled, installs from extracted sibling archives, and tests
the installed CLI. Linux also runs the independent consumer and the self-hosting check.

The compiler and testing workers coordinate retained tests before package verification. The testing
worker moves public protocol, diagnostic, source, LTO, and stage-proof journeys to compiler
integration tests. The proof source moves under their fixtures. The compiler worker removes the
originals only after those tests exist. It retains actual provisioning-count, failed-provisioning,
and missing/corrupt-bitcode unit tests with a small self-contained isolated package fixture.
The retained tests cannot require the unpublished helper. Pure compiler tests stay in place.

The moved protocol test does not assert the private unit-test-only provisioning counter. The
dedicated cache unit test still proves actual driver builds. The integration owner verifies
`cargo test --lib` against unpacked compiler sources with only extracted-sibling patches.

Product metadata and documentation are committed as `a95bdd9`. The seven packages use `0.1.0`;
the unpublished helper stays at `0.0.1`. Versionless helper dev-dependencies and integration tests
are absent from normalized product manifests and archive test targets. Retained compiler unit-test
cleanup is integrated as `324ef2f` and `303a3b2`, with 67 compiler tests passing locally.

The required command `cargo package --workspace --exclude cargo-optic-test-support --locked`
passes verification for all seven archives locally. An earlier attempt with `--offline` hit Cargo
1.98.1's internal temporary-registry checksum error. The required command works with registry access;
there is no custom registry or disabled-verification workaround. Installed runtime, external consumer,
self-hosting, and final both-host CI remain pending.

The installation worker is Arendt, using `/tmp/optic-mvp-install` from accepted B. The integration
owner also adds a concise checked-in testing guide and connects installation checks to both CI hosts.

The complete assembled revision is `373e6c8`. The installation worker passed all seven archive
verifications, 48 unpacked compiler tests, the installed CLI journey, independent API consumer, and
Linux self-hosting through `CaptureId::generate`. The integration owner is repeating the installation
workflow from this exact assembled revision. The repository suite now has 228 passing tests after
extracting the private bitcode-failure case into its own parent/child test.

Fresh independent reviewers Harvey and Hooke inspect correctness and Rust style in separate detached
worktrees at `373e6c8`. Neither implemented this code. The exact-revision CI gate now includes
installation checks on Linux and macOS. Review results and final CI remain required before merge.

The integration owner's installation run exposed an ancestor-config leak in test setup. Packaging
from the checkout found an ancestor Cargo `rustc-wrapper = "kache"` setting, although the cleared
environment omitted that executable. Run packaging from the private temporary root with an explicit
checkout manifest path. Cargo's configuration search then starts outside the user's checkout tree.
Fixture copies also use explicit checkout paths. This refines test isolation, not product wrapper
policy. Repeat the complete installed workflow after the correction.

The corrected run passes archive verification, 48 unpacked compiler tests, and installed CLI/API
journeys. Self-hosting in the live checkout still inherits an ancestor custom linker configuration,
whose executable is intentionally absent from the isolated PATH. Refine self-hosting to use a
`git archive` snapshot of the exact committed product source under the private root. This preserves
the user's proposed real-source test without configuration overrides or changes to their store.
Repeat the full script after this final isolation correction.

## Independent review corrections

Harvey found one correctness gap: a durable manifest could retain instance placements while removing
their LLVM modules and artifacts, then claim `Collected` evidence. Require each placement CGU to
exist in collected module identities. Keep modules without placements and `NotCaptured` valid.
Add constructor, deserialization, and candidate-read regressions; make existing test fixtures obey
the complete graph invariant.

Hooke identified four required quality corrections: use readers that actually split LLVM test input,
replace raw protocol decision codes with shared named constants, extract repeated disassembly/body
comparison from the nested stage-proof test, and document `InstanceRef`'s text/Serde contract.
The integration owner handles durable validation and reference docs. Separate workers handle indexer
test input, driver protocol names, and stage-proof structure. Both reviewers recheck their findings.

All corrections are integrated as `0affc0d`, `69da43e`, `6d84ad1`, and `d2ebd20`. Snapshot isolation is
`3922e35`. The final candidate `514b8a0` also corrects one link found by private rustdoc checks.
All 236 workspace tests, formatting, Clippy, and public/private rustdoc pass locally. The two reviewers
are checking the complete correction diff against their original findings.

The full installed workflow passes at `d2ebd20`, with evidence in `/tmp/optic-install.0Tnsjkqs`.
All seven archives verify, all 54 unpacked compiler tests pass, and installed CLI/API journeys pass.
Self-hosting captures `optic_records::capture_id::CaptureId::generate`, reads its source and optimized
LLVM, and reuses capture `ssrzmzwvtokxvoqxoquksspnutzopmur` with the original completion time.
The final candidate has only a rustdoc-link correction after this run. Repeat its installed gate and
wait for [CI run 33999328890](https://github.com/connortsui20/optic/actions/runs/33999328890).

Both independent reviewers approve `514b8a03c2981a1df45ef963bc75002932a9f883` with no remaining
findings. Harvey verified the graph invariant and corruption regression. Hooke verified all four
style corrections. Neither reviewer authored the implementation or its corrections.

The exact-candidate installed repeat passes, with evidence in `/tmp/optic-install.I30jpiuX`.
All seven archives, 54 unpacked compiler tests, installed CLI/API checks, and self-hosting pass.
Self-hosting returns `vqrqtnlnsoupvyovqpkprontwkrurwvx:691` for `CaptureId::generate` and preserves
completion time `2026-09-05T23:42:55.794Z` on reuse. Final macOS CI remains the last merge gate.

Final [CI run 33999328890](https://github.com/connortsui20/optic/actions/runs/33999328890) passes all
jobs on `514b8a03c2981a1df45ef963bc75002932a9f883`: Linux and macOS workspace tests and installed
verification, plus formatting, Clippy, and rustdoc. GitGuardian also passes. Every verification gate
is complete. The integration owner can now squash-merge PR #17 under the user's authorization.

## Additional acceptance

The user suggested exercising Optic on its own source during implementation. Add the single Linux
self-hosting smoke test specified in [installation](installation.md#ci-and-completion) to C.
Focused fixtures still own exact behavior, and archive-only journeys still own package isolation.
