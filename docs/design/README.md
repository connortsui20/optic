# Design documents

The `planning` branch owns product scope, behavior contracts, decisions, and acceptance history.
The `main` branch owns the implementation and current contributor documentation. Together, these
branches provide the context for a review without prior conversation history.

Start a full review with [the review guide][review] on `main`.

The MVP is complete through [PR #17](https://github.com/connortsui20/optic/pull/17).
[PR #18](https://github.com/connortsui20/optic/pull/18) corrected two regressions at `c94cc22`.
The contracts describe the completed MVP. They do not authorize deferred features.

## Current MVP contracts

Read these documents in order:

1. [Cargo Optic plan](PLAN.md).
2. [MVP architecture](mvp-architecture.md).
3. [Capture reuse](capture-reuse.md).
4. [Narrow show](show.md).
5. [Test strategy](test-strategy.md).
6. [Packaged verification](installation.md).

## Completed delivery and acceptance

- The [MVP plan](mvp-plan.md) and [foundation](stabilization.md) record completed work and its gates.
- The [agent workflow](agent-workflow.md) records the completed implementation and review process.
- The [show interfaces](show-interfaces.md) record the interface decisions for Checkpoint B.
- The [acceptance index](acceptance-evidence.md) maps required behavior to tests on `main`.
- The [execution ledger](execution.md) records checkpoint decisions, revisions, and merge results.
- The [review follow-up](review-follow-up.md) records PR #18 and its verification.

Old branch names and temporary paths identify past work. They are not setup instructions or promised
downloads. Passing results apply to their recorded revision, not automatically to a later checkout.

## Optional research

The remaining design files and `docs/research/` preserve earlier experiments and possible future work.
The separate `prototype` branch is optional experimental code, not a specification or a feature-parity
target. Neither that branch nor its capabilities are prerequisites for reviewing the MVP.

A future feature needs an active plan before implementation. Historical requirements do not expand
the current contracts, even when an older document calls them required or implemented.

[review]: https://github.com/connortsui20/optic/blob/main/docs/review.md
