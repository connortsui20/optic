# Cargo Optic plan

This document defines the implementation order for Cargo Optic. The `planning` branch is the source
of truth for planned work. The `main` branch is the source of truth for implemented behavior.

Cargo Optic is a prototype until the first public release. The prototype can change commands,
library APIs, records, protocols, and stored data without compatibility support.

## Product goal

Cargo Optic records compiler evidence for one real Cargo target. A user can find a concrete Rust
instance and inspect the source and LLVM IR from that build.

The first complete workflow is:

```console
cargo optic capture -p my-crate --lib --release
cargo optic list-captures
cargo optic find --capture CAPTURE_REF kernel
cargo optic show --instance INSTANCE_REF --output source
cargo optic show --instance INSTANCE_REF --output llvm
```

An unchanged second capture will reuse complete evidence after Cargo reports that the selected
analysis is fresh.

## Development goal

The prototype ships small features at high quality. Simple code makes this quality practical.

The implementation obeys these rules:

- Implement one user-visible claim at a time.
- Use a direct implementation before an abstraction.
- Add an abstraction after a second real caller or implementation appears.
- Keep the current crate boundaries because each crate has known future consumers.
- Return a clear error for an unsupported environment or unexpected input.
- Do not add speculative recovery, compatibility, portability, or optimization code.
- Add support for an unusual case with a fixture that reproduces the case.
- Measure a performance problem before the implementation adds performance complexity.
- Prefer deletion when code does not protect a current contract.

## Sources of truth

The planning documents have this order:

1. This document defines the goals, order, and completion rules.
2. [MVP architecture](mvp-architecture.md) defines the intended product boundaries.
3. [MVP plan](mvp-plan.md) defines each product slice.
4. [Stabilization plan](stabilization.md) defines the pause after the walking MVP.
5. [Test strategy](test-strategy.md) defines the executable contracts.
6. [Agent workflow](agent-workflow.md) defines autonomous implementation and review.

The other design documents describe the earlier prototype or possible future work. They provide
evidence, but they do not authorize implementation. The [future-work document](future-work.md)
contains ideas that need a new plan before implementation.

## Program phases

### Phase 1: Walking MVP

The walking MVP supports `capture`, `list-captures`, and `find`. It proves the Cargo connection,
exact-version driver, durable publication, and concrete-instance search.

Current stack:

- [x] Capture one selected Cargo target and list completed captures in PR #5.
- [x] Add durable records for concrete compiler instances in PR #7.
- [x] Collect concrete instances with an exact-version driver in PR #8.
- [x] Remove unsupported compiler-wrapper compatibility in PR #11.
- [x] Merge the driver workflow clarification in PR #14.
- [x] Merge complete compiler-capture publication in PR #9.
- [x] Merge user-visible concrete-instance search in PR #6.

PRs #14, #9, and #6 merge in that order. This stack uses one exception to the future CI rule
because the repository does not have CI yet.

The [stabilization plan](stabilization.md) starts immediately after this stack merges.

### Phase 2: Stabilization

Stabilization pauses product features. It adds the minimum structure that lets agents change the
prototype without inventing behavior.

The phase has five outcomes:

1. Linux and macOS CI protect the repository.
2. Current architecture and support limits exist on `main`.
3. Integration tests use one small, hermetic fixture harness.
4. The walking-MVP contract has complete black-box coverage.
5. A test-backed cleanup removes unnecessary complexity from the implementation.

The phase does not add a test DSL, snapshot framework, compatibility layer, platform abstraction,
or generalized orchestration system.

### Phase 3: Captured source

The source slice stores source that belongs to a recorded definition. It reads the stored snapshot
instead of the current checkout.

The slice accepts source only from approved local package roots. Missing source remains a valid
availability result.

Completion criterion:

> A user can show the captured source for one selected compiler instance.

### Phase 4: Exact LLVM IR

The LLVM slice emits optimized LLVM IR and records byte ranges for function bodies. An exact raw
symbol connects an instance to an LLVM body.

The slice distinguishes an available body from an optimized-away body. Similar display text does
not create an evidence relationship.

Completion criterion:

> A user can show exact LLVM evidence or learn that the instance has no standalone body.

### Phase 5: Capture reuse

The reuse slice uses Cargo as the authority for freshness. It does not reconstruct Cargo's private
fingerprint rules.

A request key selects a completed capture and its saved analysis fingerprint. Cargo Optic returns
the selected capture only when Cargo reports that analysis as fresh.

The same slice caches the exact-version driver by compiler identity, driver source digest, and
protocol version.

Completion criterion:

> An identical capture reuses complete evidence without rebuilding the selected target or driver.

### Phase 6: First release

The release slice completes package metadata, installation documentation, public errors, and an
outside-workspace installation test. It does not add another product feature.

Completion criterion:

> A crates.io installation completes the documented workflow outside the source workspace.

## Work that follows the MVP

Later product work starts only after the first release meets its completion criterion. Each new
feature needs a decision-complete plan on the `planning` branch.

Potential later work includes:

- Optimization remarks.
- MIR, assembly, objects, and linked-product evidence.
- Cross-capture identity and comparison.
- Attribution for compiler transformations.
- Capture labels, retention, removal, and garbage collection.
- Dependency capture.
- Foreign stores and portable evidence bundles.
- A TUI, server, or editor integration.

This list does not define an order. Prototype evidence in [future work](future-work.md) can inform a
later plan.

## Completion policy

Each implementation slice needs all of these results before merge:

- The change matches an approved implementation packet on `planning`.
- The public API and durable data changes match the packet.
- Required unit, integration, and CLI tests pass.
- Linux and macOS CI pass after the CI phase lands.
- Clippy and rustdoc report no warnings.
- Rust formatting is clean.
- An independent review reports no unresolved correctness or design findings.
- The complete diff contains no speculative support or unnecessary abstraction.

An agent can merge a conforming pull request without maintainer review. An agent must stop when the
implementation needs a contract change that the planning documents do not authorize.

## Planning branch maintenance

The `planning` branch contains documentation commits on top of `main`. It does not contain a second
implementation.

Before implementation starts, the applicable goals and subgoals must exist on `planning`. After an
integration milestone, rebase `planning` onto the new `main` and update the status lists.

The branch can use rewritten history. Force updates must use a lease. The document content is the
durable record, not the commit identifier.
