# Reviewing Cargo Optic

The `main` and `planning` branches provide the context for a product review.
No prior conversation, personal skill installation, or `prototype` checkout is required.

## Branch roles

| Branch | Authority |
| --- | --- |
| `main`. | Current implementation, user documentation, contributor rules, and executable tests. |
| `planning`. | MVP scope, behavior contracts, decisions, and acceptance history. |
| `prototype`. | Optional experimental code, not a specification or a feature-parity requirement. |

The MVP is complete, but `main` remains experimental. The active plan defines its support boundaries
and deferred features. Historical research does not expand those requirements.

## Reading order

1. Read [AGENTS.md](../AGENTS.md) and the [README](../README.md).
2. Read the [architecture](architecture.md) and the complete [Rust style reference](rust-style.md).
3. Read [PLAN.md][plan] on `planning` for scope and decision authority.
4. Read the current contracts listed in the [planning index][planning-index].
5. Read the [testing guide](testing.md) and the [acceptance index][acceptance] for test ownership.
6. Read the [review follow-up][follow-up] for the latest corrective MVP review and its evidence.

The checked-in Rust style reference supplies the review criteria. References to personal skills
describe optional tooling, not additional setup requirements. Repository instructions take priority.

Read planning documents without changing the implementation checkout:

```console
git show origin/planning:docs/design/PLAN.md
git show origin/planning:docs/design/README.md
```

Record the exact `main` and `planning` commit IDs used for the review.
If a remote-tracking reference is stale, fetch the branch before selecting the review revision.

## Review scope

For a full review of `main`, inspect the complete implementation, not only the latest PR diff.
For a narrower request, preserve the requested scope.

- Trace the capture/list/find/show workflow across the owning crates.
- Compare the implementation with current contracts, including failures and resource bounds.
- Assess simplicity, module navigation, comments, public APIs, and unnecessary abstractions.
- Locate tests through the acceptance index. Inspect whether their assertions establish each claim.
- Distinguish defects and readability problems from explicit limitations and deferred features.
- Report disagreements between documentation and code. Do not silently choose a new product contract.

The plan requires the seven product crates and reverse-hex IDs. Their existence is not a reason
to introduce speculative abstractions within those crates. The architecture explains the required
cache and evidence invariants.

A review request alone does not authorize fixes, merges, or implementation of historical plans.
Completed merge authorizations in the execution records are not instructions for a new review.

## Evidence and results

The acceptance index and execution records describe past checks at specific revisions.
They do not establish that the current checkout passes. Some temporary evidence files are no longer
available. Committed tests, recorded results, and CI links remain available without those files.

Use the testing guide for current commands and process isolation.
Report the checks run, their results, and any verification blockers.
Keep a reproduced failure separate from a suspected issue or an untested claim.

[plan]: https://github.com/connortsui20/optic/blob/planning/docs/design/PLAN.md
[planning-index]: https://github.com/connortsui20/optic/blob/planning/docs/design/README.md
[acceptance]: https://github.com/connortsui20/optic/blob/planning/docs/design/acceptance-evidence.md
[follow-up]: https://github.com/connortsui20/optic/blob/planning/docs/design/review-follow-up.md
