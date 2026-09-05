# Stabilization plan

This phase starts after PRs #14, #9, and #6 merge. Product feature work stops until this phase is
complete.

The phase makes the walking MVP safe for autonomous changes. It uses small additions and does not
create a general development platform.

## Entry state

The walking MVP provides these commands:

```console
cargo optic capture -p my-crate --lib --release
cargo optic list-captures
cargo optic find --capture CAPTURE_REF kernel
```

The workspace already has unit and integration tests. It does not have hosted CI, shared fixture
support, current architecture documentation on `main`, or one complete behavior matrix.

## Pull request 1: establish CI

Review question:

> Does every proposed change run the existing quality checks on Linux and macOS?

Add one GitHub Actions workflow. Use the toolchain and components from `rust-toolchain.toml`.

Run these checks:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace
```

Run the workspace tests on Linux and macOS. The formatting, Clippy, and rustdoc jobs can run on
Linux.

Do not add a Cargo cache, third-party test runner, coverage service, release workflow, or scheduled
platform matrix. Add those tools only when a measured problem requires them.

Completion criterion:

> A pull request cannot auto-merge until the repository quality checks pass.

## Pull request 2: publish current contracts

Review question:

> Can a contributor understand the implemented system and its limits from `main`?

Add a concise architecture guide to `main`. It must describe:

- The CLI and library entry points.
- The responsibility of each existing crate.
- The Cargo, wrapper, driver, store, API, and CLI data flow.
- The exact-version driver process boundary and protocol.
- Capture validation and atomic publication.
- Durable record ownership and validation.
- The supported compiler and host environment.
- Explicit unsupported cases.
- The current absence of compatibility guarantees.

Add short decision records for choices that agents can otherwise reopen. The initial records cover:

- Cargo remains the authority for freshness.
- The driver compiles against the selected rustc version.
- The store publishes one complete capture with an atomic rename.
- Unexpected environments return errors instead of compatibility behavior.
- Current crate boundaries remain even when a crate has one present caller.

Add root agent instructions. The instructions must require simple code, approved contracts, tests,
the Rust style rules, and an independent review.

Completion criterion:

> An agent can identify the current contract without reading the future roadmap or Git history.

## Pull request 3: share hermetic test fixtures

Review question:

> Do integration tests start from the same isolated Cargo environment without duplicated setup?

Add one unpublished test-support crate. Existing API, compiler, and CLI tests are separate callers,
so the shared boundary has current use.

The crate owns only these test facilities:

- Creation of a temporary Cargo workspace.
- Copying of checked-in fixture source.
- Creation of an isolated Cargo home and target directory.
- Removal of ambient compiler and wrapper environment variables.
- Formatting of child-process diagnostics.
- Shared custom assertions that have more than one caller.

Keep binary discovery and product-specific assertions in the tests that own them. Do not create a
command builder, scenario language, snapshot layer, or generic filesystem library.

Migrate the existing API, compiler, and CLI integration tests. Preserve their behavior during the
migration.

Completion criterion:

> Integration tests cannot read user Cargo configuration or share build state by accident.

## Pull request 4: complete the walking-MVP contract suite

Review question:

> Do black-box tests protect every current user-visible claim and publication invariant?

Add one fixture package that contains:

- A non-generic function.
- A generic function with at least two concrete instances.
- A feature-selected target.
- A source file with a space in its name when the current implementation uses that path.

The public API and CLI tests cover:

- An empty capture history.
- One successful capture and list operation.
- Concrete-instance lookup by exact name and literal substring.
- Deterministic lookup order and the documented result limit.
- Package, target, profile, and feature selection.
- Two repeated captures before reuse exists.
- A missing target.
- Invalid Rust source.
- A driver collection error.
- No visible capture after each pre-publication error.
- Wrapper disabling with its user warning.
- Rejection of a configured or environment-selected compiler.
- Rejection of malformed current-format durable data.
- Bounded rejection of an oversized durable instance manifest.

The driver integration test crosses the real process boundary. It proves that the writer and reader
agree on the current protocol. It does not freeze private protocol bytes or promise compatibility.

Do not add tests for Windows, response files, non-executable temporary filesystems, network
filesystems, recovery, concurrency, or migration. These cases are outside the support contract.

Completion criterion:

> A simple regression in the walking MVP fails one focused test before it reaches `main`.

## Pull request 5: simplify the walking MVP

Review question:

> Is every remaining layer and edge-case branch necessary for a tested current contract?

Review the complete implementation after the contract suite lands. Apply these rules:

- Remove code for unsupported edge cases.
- Remove compatibility code for older prototype data.
- Remove one-caller wrappers that do not name a real concept.
- Keep a straight-line function when splitting it adds navigation without clarity.
- Keep an existing crate boundary because future application and subsystem callers are known.
- Replace recovery for unexpected input with one clear error.
- Replace comments that narrate code with names or structure.
- Add module or item documentation when the entry point, invariant, or rationale is not clear.
- Keep protocol constants and durable format constants documented beside their definitions.

This pull request does not change approved behavior. If deletion changes a contract, update the
planning branch before the implementation continues.

Completion criterion:

> The walking MVP contains the minimum code that satisfies its current contracts and quality gates.

## Exit review

After all five pull requests merge, run one accumulated review against `main`. The review must use
the architecture guide, behavior matrix, and Rust style rules.

The phase is complete when:

- Linux and macOS CI pass on `main`.
- All current contracts have an owning test.
- The integration tests are isolated from the developer machine.
- The architecture guide matches the code.
- The simplification review has no unresolved findings.
- The `planning` status reflects the merged state.

Only then can the captured-source slice start.
