# Cargo Optic MVP

The MVP supports one complete investigation: capture a Cargo target, find a concrete Rust instance,
and inspect its captured source or optimized LLVM output. Identical requests reuse evidence only
after Cargo validates freshness. The exact-version driver is cached too.

The user confirmed this scope on 2026-09-05. Capture, listing, search, and both caches must work
well before development starts on narrow `show`. The implementation lands as one coherent MVP
change.

## Definition of done

The following workflow must work through both the CLI and the `optic` library:

```console
cargo optic capture -p my-crate --lib --release
cargo optic list-captures
cargo optic find --capture CAPTURE_REF kernel
cargo optic capture -p my-crate --lib --release
cargo optic capture -p my-crate --lib --release --fresh
cargo optic show --instance INSTANCE_REF --output source
cargo optic show --instance INSTANCE_REF --output llvm
```

The identical second capture returns the existing capture ID and completion time. It does not
rebuild the selected target or driver, copy evidence, or add another history entry.

A changed tracked input produces a new capture with new evidence. `--fresh` forces a new analysis of
the selected target. It still reuses the compatible driver and ordinary dependency artifacts.

Every completed capture is self-contained. A compilation or storage error must never make old
evidence eligible for a newer successful Cargo build.

The installed CLI and an external library consumer must pass the same workflow outside the source
checkout. Linux and macOS CI, independent correctness review, and `$rust-style` review must pass.

## Current baseline

The baseline is `main` at `684991a`. Earlier PRs #14, #9, and #6 are merged.

| Capability | Baseline status | MVP requirement |
| --- | --- | --- |
| Capture, list, and find. | Implemented. | Preserve behavior and complete failure coverage. |
| Concrete rustc instances. | Implemented through an exact-version driver. | Preserve exact compiler identity and symbols. |
| Capture reuse. | Absent. Every request forces selected-target work. | Reuse only after Cargo freshness confirmation. |
| Driver reuse. | Absent. Every request rebuilds the driver. | Cache by compiler identity and complete driver input. |
| Durable read bounds. | Absent from JSON readers. | Enforce documented limits before deserialization. |
| Source and LLVM evidence. | Absent. | Add after the cache checkpoint. |
| Instance reference and show. | Absent. | Add only with narrow show. |
| CI. | PR #16 is open and its Linux/macOS checks pass. | Incorporate it into the integrated MVP. |
| Installation. | All packages are unpublished. | Verify packaged CLI and library consumption. |

A passing baseline test suite does not establish the missing requirements. Status changes require
the acceptance evidence defined in the test strategy.

## Reading order and authority

1. This document defines product scope and the completion gate.
2. [MVP plan](mvp-plan.md) assigns work and orders the integration checkpoints.
3. [MVP architecture](mvp-architecture.md) fixes subsystem boundaries and shared contracts.
4. [Foundation](stabilization.md) defines the preparation and simplification work.
5. [Capture reuse](capture-reuse.md) defines the cache state machine.
6. [Narrow show](show.md) defines source and LLVM evidence.
7. [Test strategy](test-strategy.md) defines fixtures, observations, and acceptance scenarios.
8. [Installation](installation.md) defines the packaged deliverable.
9. [Agent workflow](agent-workflow.md) defines parallel work and independent review.

These documents own different decisions. Link to the owner instead of repeating a contract.

Other documents in this directory and `docs/research/` retain earlier prototype research or future
designs. Their capabilities are not additional MVP requirements. The active documents take
precedence when old research differs.

## Simplicity and quality

`main` is a prototype integration branch until release. Commands, records, protocols, and Rust APIs
can change together. No migration or backward-compatibility code is required.

Keep the existing seven product crates. Known future callers justify those subsystem boundaries.
Inside each subsystem, apply the full `$rust-style` rules, including structural restraint.

Simple code must still enforce the claims that the user relies on. Cache invalidation, exact
compiler evidence, bounded durable reads, and atomic publication are required correctness work. They
are not optional edge-case support.

Do not implement Windows support, response-file compatibility, noexec mount handling, custom
compiler composition, concurrent capture, or crash recovery. Unexpected unsupported cases return
errors. Ignoring such a case is valid only when it cannot change the result's meaning.

Cache reuse is a required product behavior. It does not need a performance benchmark to justify its
existence. Further optimization needs a measured bottleneck. No timing threshold defines
correctness.

## Delivery order

The implementation has three internal checkpoints:

1. **A: Reliable and efficient capture/list/find.** Establish the test harness and finish both
   caches.
2. **B: Narrow show.** Add references, captured source, optimized LLVM, and final-format cache
   tests.
3. **C: Release candidate.** Verify installation, finish documentation, and review the complete
   code.

These checkpoints are not separate PR requirements. The integration owner can restructure code
across crates during this effort. The final review covers the assembled implementation.

## Deferred features

The following features are outside this MVP:

- Build-and-query `show QUERY` and automatic capture from a query.
- MIR, assembly, objects, pre-optimization LLVM output, and optimization remarks.
- Cross-capture identity, comparison, or transformation attribution.
- Automatic dependency capture.
- Store federation, compression, deduplication, retention, and garbage collection.
- Recovery of interrupted work or concurrent capture.
- JSON Lines, cancellation APIs, a TUI, a server, or an editor integration.
- Compatibility with old stored data or old API versions.

A dependency instance present in the selected compilation remains legitimate selected-target
evidence. Narrow source capture initially supports only local definitions with approved
compiler-loaded spans.

## Persistence and execution boundary

Persist the complete plan on `planning` before implementation starts. The planning branch remains
docs-only on top of `main`. Its history can be rebased using leased force updates.

Current behavior belongs in documentation on `main`. Planned behavior belongs here until the
implementation passes its checkpoint. The status in this plan must distinguish implemented, tested,
and merged work.

The user subsequently authorized implementation with “Implement the plan.” Execution is underway
in draft PR #17. The [execution ledger](execution.md) records progress and verification without
changing the checkpoint requirements. Automatic merging remains authorized after the final quality
gate. Registry publication needs a separate release instruction.
