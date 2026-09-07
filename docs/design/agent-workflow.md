# MVP implementation and review

This document records the workflow used for the completed MVP. Its assignments, branch names, and
landing authorization belong to that effort, not to a new task. Current contributor instructions
live in [AGENTS.md](../../AGENTS.md). The [execution ledger](execution.md) records the results.

One integration owner delivers the complete MVP in one implementation pull request. Internal
checkpoints establish correctness before dependent work starts. They do not require separate merges
or repeated maintainer review.

The [MVP plan](mvp-plan.md) defines the checkpoints. Implementation starts only after this plan and
its linked contracts are committed and pushed to `planning`.

## Roles and ownership

Workers use the same base revision and separate Git worktrees. High reasoning is appropriate for
compiler and cache correctness, storage publication, and final review.

| Role | Write ownership | Required result |
| --- | --- | --- |
| Integration owner | Capture orchestration, API, CLI, workspace manifests, CI, documentation, and final integration. | One complete workflow with consistent public contracts. |
| Compiler worker | Compiler crate and standalone driver, including compiler tests. | Cargo freshness, driver cache, and later source/LLVM collection. |
| Storage worker | Records and store crates, including their unit tests. | Validated evidence, cache eligibility, and atomic publication. |
| Test worker | Unpublished test-support crate and cross-crate integration test files. | Isolated fixtures and independent acceptance scenarios. |

Evidence-query work starts after the cache checkpoint. The integration owner can assign the evidence
crate to another worker once its record and store interfaces are fixed.

Workers do not share a writable worktree. The integration owner edits root manifests and the lock
file because all workers can otherwise conflict there. Workers report the dependencies they need.

## Preparation

1. Record the integration base and planning commit in the implementation pull request.
2. Read the applicable contracts and all affected consumers.
3. Read the complete [Rust style reference](../rust-style.md) before designing or changing Rust code.
4. Establish shared signatures and record ownership before parallel edits begin.
5. Give each worker its allowed files, required tests, dependencies, and excluded work.

The implementation used `ct/complete-mvp` and incorporated the CI commit from PR #16.
PR #17 merged the MVP, and PR #16 closed as superseded. Both task branches are obsolete.

## Execution checkpoints

The integration owner and compiler/storage workers agree on the cache state machine before
implementation. The test worker can prepare independent fixtures while those interfaces settle.

At each checkpoint, the integration owner incorporates worker commits and runs the affected suite.
Workers then update their bases to that integrated revision. Dependent development uses that base.

Checkpoint A completes capture, listing, search, both caches, and their failure tests. Checkpoint B
adds narrow `show` and repeats cache acceptance with the final evidence format. Checkpoint C proves
installation and reviews the complete codebase.

A worker handoff records:

- The base and result commit identifiers.
- Changed files and public contracts.
- Tests run and their outcomes.
- Remaining failures or unsupported inputs.
- Any difference from the planning documents.

The integration owner reads each diff before incorporation. A worker report does not replace that
inspection. Do not duplicate a worker's implementation in the integration worktree.

## Rust style and simplicity

The explicit decision to retain current crate boundaries takes precedence over the skill's rule
against predicting future callers. That exception does not authorize new internal abstractions.

Apply the skill's restraint rules throughout:

- Keep one concept per module and a direct path from its entry point to the implementation.
- Use `foo.rs` for a leaf and `foo/mod.rs` for a module with children.
- When extraction adds no useful boundary, keep straight-line functions intact.
- Introduce a type distinction for plausible, harmful confusion.
- Validate values at construction and deserialization, then trust those invariants internally.
- Use concrete types and standard library interfaces before adding traits or wrappers.
- Document public fields, variants, entry points, and non-obvious private contracts.
- Explain compiler-sensitive choices and their failure modes beside the relevant code.
- Document format markers, versions, and durable input limits beside their constants.
- Remove stale TODOs, unnecessary layers, and unsupported compatibility code deliberately.

Main contains project instructions and the complete checked-in Rust style reference.
Reviewers do not need a personal skill installation or prior conversation history.
Available skills can supplement the reference, but cannot expand the authorized task.

## Independent review

Use two independent reviewers once the full MVP is assembled. Neither reviewer implements the
changes it reviews.

The correctness reviewer examines the full integration diff and final contracts. Its main questions
concern stale cache eligibility, record agreement, and evidence claims. It also examines
installation and failure tests.

The Rust reviewer reads the full final implementation of affected concepts under `$rust-style`. It
examines module navigation, types, error boundaries, comments, tests, and unnecessary layers.

Give reviewers the source and plan before the author's summary. Findings must name a concrete
contract violation, defect, readability problem, or unnecessary mechanism.

Resolve all actionable findings. Re-run affected tests after a correction. Re-review affected
contracts when a correction changes them. Run the complete quality gate on the final revision.

## Landing

The user authorized automatic merging of the conforming MVP after its checks and independent reviews.
The integration owner merged PR #17. That completed authorization does not turn a later review request
into permission to change code or merge a new feature.

The check result must correspond to the revision being merged. Results from an earlier revision or
another worktree do not establish that the final revision passes.

The merge record includes the planning revision, test results, review results, known limits, and
final implementation revision. Rebase `planning` onto the resulting `main` and update its status.

Rewriting prototype history remains allowed, but normal implementation does not require rewriting
`main`. Preserve other contributors' work and use leases for authorized branch rewrites.

## Decision boundaries

The integration owner resolves routine names, file placement, and local implementation details.
Record refinements on `planning` before dependent work starts. Use the existing contract to resolve
decisions that do not change product behavior.

Request a product decision only for a new feature, support promise, compatibility guarantee, or
contradictory user-visible behavior. Continue independent work while that decision is pending.

Do not publish to a package registry as part of automatic PR merging. Registry publication needs an
explicit release instruction. The MVP deliverable includes the verified release candidate.
