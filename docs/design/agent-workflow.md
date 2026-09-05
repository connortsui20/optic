# Agent workflow

This workflow lets agents implement approved Cargo Optic work without routine maintainer review.
The planning documents constrain each agent's decisions.

## Planning gate

Do not start an implementation branch until the `planning` branch contains an implementation
packet for the change.

Each packet must define:

- One user-visible claim or one repository-quality claim.
- The current problem.
- The public API changes.
- The durable record or protocol changes.
- The data and control flow.
- Supported inputs.
- Explicit unsupported inputs.
- Error and publication behavior.
- Required tests.
- Excluded work.
- One completion criterion.

The packet must contain enough detail that an implementation agent does not select product
behavior.

## Implementation role

One agent owns one pull-request layer. The agent reads the complete packet, current architecture,
root instructions, and applicable style guidance before editing code.

The implementation agent must:

1. Inspect the complete base and up-stack consumers.
2. Add or update tests for the approved claim.
3. Implement the smallest design that passes the tests.
4. Document public contracts and non-obvious private invariants.
5. Run the required local checks.
6. Inspect the complete diff against the pull-request base.
7. Remove code that does not support the packet.

The implementation agent must not:

- Add behavior for an input that the packet excludes.
- Add compatibility for an earlier prototype format.
- Add a trait for one implementation.
- Add a wrapper for one caller unless it names a current invariant.
- Weaken a test to accept behavior outside the packet.
- Change another stack layer to avoid a proper rebase.
- Treat a future caller as proof of a current abstraction.

## Supporting agents

Supporting agents can inspect one bounded concern. Useful concerns include Cargo behavior, a store
invariant, a test fixture, a public API, or a complete style pass.

A supporting agent reports evidence and does not change the product contract. The implementation
agent remains responsible for the complete diff.

Do not assign several agents to edit the same module in parallel. Parallel work is useful only when
the boundaries and outputs do not overlap.

## Independent review role

The review agent reads the packet and the complete diff from the pull-request base. It does not
start with the implementation agent's summary.

The review covers:

- Correctness against the packet.
- Unsupported behavior that entered the implementation.
- Missing errors or publication guarantees.
- Public API and durable data changes.
- Cross-process and filesystem boundaries.
- Test quality and missing contract cases.
- Rust readability, documentation, and structural restraint.
- Changes to downstream stack layers.

The review agent reports only actionable findings. The implementation agent resolves each finding
or records why the packet makes it invalid.

## Merge gate

The existing walking-MVP stack has one exception because it predates repository CI. It can merge
after all local checks and one independent accumulated review pass.

After the CI workflow lands, an agent can auto-merge a pull request when:

- The pull request matches its planning packet.
- Required Linux and macOS jobs pass.
- Formatting, Clippy, and rustdoc jobs pass.
- An independent review has no unresolved findings.
- The branch includes the current base.
- The pull-request description states the claim and main limit.

Use squash merges for implementation pull requests. Rebase the remaining stack after each lower
layer merges.

## Stop conditions

Stop implementation and request a decision when:

- The public behavior needs to differ from the packet.
- A durable format needs a new compatibility promise.
- A new supported platform or compiler environment enters scope.
- A crate boundary or dependency direction must change.
- Correctness requires a new recovery or concurrency policy.
- A test exposes two valid product behaviors and the packet does not choose one.

A difficult implementation is not a stop condition. Reduce the implementation to the approved
claim before adding machinery.

## Planning maintenance

After each merge, update the status on `planning`. Record a newly discovered limit in the applicable
packet or future-work document.

Rebase the docs-only `planning` branch onto `main` after an integration milestone. Use a leased
force update because rebasing changes commit identifiers.

Current behavior belongs in documentation on `main`. Future behavior remains on `planning` until
its implementation merges.
