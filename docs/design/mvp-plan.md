# MVP implementation plan

This plan finishes the product defined in [PLAN.md](PLAN.md). It replaces the previous sequence of
small stabilization and feature PRs. One integration branch carries the complete implementation.

## Work ledger

The user-visible completion criterion applies to the whole MVP. The following unchecked items are
required work, including tests and documentation.

| ID | Work | Dependency | Acceptance owner |
| --- | --- | --- | --- |
| F1 | Incorporate CI and establish reproducible toolchain checks. | Current main. | Foundation. |
| F2 | Add shared test fixtures and move process tests to isolated children. | F1. | Test strategy. |
| F3 | Bound durable JSON reads and writes, and test publication failures. | F2. | Architecture and store tests. |
| C1 | Cache the exact-version driver at a stable path. | F2. | Capture reuse. |
| C2 | Implement Cargo freshness, cache eligibility, and failure invalidation. | F3 and C1. | Capture reuse. |
| C3 | Expose capture reuse and `--fresh` through the API and CLI. | C2. | Checkpoint A journey. |
| S1 | Add capture-scoped instance references and bounded evidence storage. | Checkpoint A. | Narrow show. |
| S2 | Capture exact source spans and stored source text. | S1. | Source tests. |
| S3 | Collect optimized LLVM artifacts and index exact bodies. | S1. | LLVM tests. |
| S4 | Connect narrow show through the evidence API and CLI. | S2 and S3. | Checkpoint B journey. |
| R1 | Verify the packaged CLI and external library consumer. | Checkpoint B. | Installation. |
| R2 | Finish current architecture, limitations, and contribution documentation. | All implementation. | Independent reviews. |
| R3 | Review, simplify, verify, and land the complete MVP. | R1 and R2. | Final quality gate. |

## Foundation subgoals

- [ ] Incorporate the CI commit from PR #16 into `ct/complete-mvp`.
- [ ] Pin the development toolchain to the tested Rust release, initially `1.98.1`.
- [ ] Run formatting, Clippy, documentation, and Linux/macOS tests from that toolchain.
- [ ] Format the standalone driver's source files as well as Cargo workspace files.
- [ ] Add the unpublished test-support crate with actual API, compiler, and CLI consumers.
- [ ] Keep all integration environment changes inside child processes.
- [ ] Preserve the real rustup installation while isolating Cargo configuration and target state.
- [ ] Add bounded durable readers and prevent writers from publishing unreadable oversized records.
- [ ] Add deterministic tests for pre-commit storage errors and invalid durable input.
- [ ] Record current architecture and the Rust style reference for contributors.

The [foundation document](stabilization.md) and [test strategy](test-strategy.md) fix these details.
Do not create a generic scenario framework, filesystem backend, or command abstraction.

## Checkpoint A: capture, list, find, and reuse

- [ ] Keep request selection, feature forwarding, invocation directory, and wrapper policy correct.
- [ ] Build the driver once per compatible compiler and complete driver source digest.
- [ ] Reuse a stable wrapper executable and stable analysis marker on ordinary matching requests.
- [ ] Ask Cargo to evaluate freshness for every attempted evidence reuse.
- [ ] Keep store output outside Cargo's default build-script tracking through store initialization.
- [ ] Distinguish a selected-target compilation from a positively identified fresh Cargo result.
- [ ] Never compile a token already associated with a completed capture.
- [ ] Install the new candidate immediately before atomic capture publication.
- [ ] Return the same capture ID and timestamp on reuse without duplicate publication.
- [ ] Generate a new analysis marker for `--fresh` without rebuilding the compatible driver.
- [ ] Generate a new marker after a stopped stale probe, including every failed-attempt retry.
- [ ] Keep previous completed captures searchable after a later request fails.
- [ ] Preserve normal dependency reuse and Cargo target-directory configuration.
- [ ] Expose `Captured` versus `Reused` in CLI output and typed API results.
- [ ] Pass the full cache journey, invalidation matrix, and failure sequence on Linux and macOS.
- [ ] Inspect all touched code for unnecessary layers under `$rust-style`.

The [capture-reuse contract](capture-reuse.md) owns the state machine. A cache hit based only on a
request hash, a missing manifest, or successful Cargo exit does not satisfy this checkpoint.

Checkpoint A ends when a fresh process can capture, list, find, reuse, change source, and recapture.
The tests must prove actual compiler and driver work. Elapsed time is not sufficient evidence.

No source or LLVM implementation starts before this checkpoint passes. Research and fixture design
can run in parallel because they do not change the implementation.

## Checkpoint B: narrow show

- [ ] Add instance references without deriving them from display names or symbols.
- [ ] Have find return and print references that select the same stored instance after sorting.
- [ ] Store source and LLVM artifacts with the capture's existing atomic publication boundary.
- [ ] Capture source text from the compiler's source map with matching byte offsets.
- [ ] Record source unavailability when the compiler cannot establish an approved exact span.
- [ ] Collect the supported optimized LLVM stage without changing the chosen optimization
      configuration.
- [ ] Index exact symbols and return every applicable standalone body in deterministic order.
- [ ] Report missing exact evidence without guessing that LLVM optimized the body away.
- [ ] Expose streaming source/LLVM readers through the library and explicit `show --instance`.
- [ ] Keep stdout useful for piping and stderr for diagnostics and unavailable-evidence messages.
- [ ] Increment the evidence version so instance-only captures cannot satisfy complete-evidence
      requests.
- [ ] Repeat cold, warm, changed-source, failed-publication, and `--fresh` tests with complete
      evidence.
- [ ] Verify that stored source and LLVM reads do not rebuild the target or use current source
      bytes.

The [narrow-show contract](show.md) owns the reference shape, availability, stage selection, and
reader semantics. Do not add convenience query selection or another compiler output mode.

## Checkpoint C: installable release candidate

- [ ] Package the complete embedded driver source and required assets.
- [ ] Verify an installed CLI outside the repository and its build directories.
- [ ] Verify the `optic` API from a separate Cargo consumer.
- [ ] Verify cold capture, reuse, find, and both show outputs through that installed build.
- [ ] Verify actionable errors for missing required toolchain components.
- [ ] Document installation, the full workflow, supported environments, and known limits.
- [ ] Reconcile public APIs, record constants, help text, tests, and the architecture guide.
- [ ] Complete independent correctness and Rust-style reviews of the whole implementation.
- [ ] Run the final CI suite on the exact revision to merge.
- [ ] Squash-merge the complete MVP and update `planning` with its evidence and status.

The [installation document](installation.md) owns packaging and the external-consumer test. The
[agent workflow](agent-workflow.md) owns assignments, review, and merging.

## Complexity audit

Perform the audit after the relevant behavior works and again on the final implementation. Each new
type, helper, module, dependency, and error branch needs a current contract or reader benefit.

Keep the existing crate boundaries and reverse-hex IDs. Allow API and format changes across the
integrated branch without migration machinery. Prefer direct functions over one-caller wrappers,
unnecessary borrowed mirrors, and traits with one implementation.

Do not impose line-count, coverage-percentage, or comment-ratio targets. Review whether a
contributor can understand the entry points, invariants, and failure paths in one pass.

## Completion evidence

The final handoff records the tested compiler and hosts, CI run, installed workflow result, review
findings and resolutions, and remaining explicit limitations.

A green test suite for only capture/list/find is Checkpoint A. It is not the completed MVP. The MVP
is complete only after narrow show and the installed workflow pass Checkpoints B and C.

## Planning review

The 2026-09-05 planning pass compared these contracts with `main` at `684991a` and the open CI PR #16.
Specialist investigations covered cache publication, compiler evidence, test isolation, and packaging.
Two independent reviewers examined the assembled plan under the complete Rust style rules.

Their findings added explicit capture-orchestrator ownership, unsupported-LLVM tests, stored Cargo
profile observations, and protection against default build-script tracking of store output.
Both reviewers verified their findings were resolved. Local Markdown links and anchors pass validation.

This is planning evidence, not implementation acceptance. The optimized artifact-stage mapping still
requires the compiler reproduction specified in the show contract.
