# MVP foundation

This foundation is part of the integrated MVP, not a separate five-PR stabilization stack. It
supplies the checks needed before capture reuse and narrow show can be trusted.

## F1: Reproducible CI

Incorporate the existing CI proposal, PR #16, into the MVP branch. Pin the development toolchain to
the first tested release, Rust 1.98.1, with rustfmt, Clippy, rustc-dev, and llvm-tools.

Run these commands from a clean checkout:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace
```

Also run rustfmt checking on standalone driver sources that Cargo's module discovery does not
include. Keep that file selection in one checked-in verification script.

Run tests on Linux and macOS. Formatting, Clippy, and rustdoc can run on Linux. Checkpoint C adds
the packaged-installation journey to CI. Do not add CI caching, coverage quotas, flaky-test retries,
or a second test runner before a measured need.

A development toolchain pin is not a promise to support all compiler versions. Publish the actual
tested compiler and host combinations.

## F2: Shared test support

Create one `publish = false` test-support crate with compiler, API, and CLI test consumers. Use
path-only, versionless dev-dependencies. The [installation plan](installation.md) defines how to
keep this helper out of distributable packages.

The helper owns fixture copying, temporary directories, command environment application, and shared
failure diagnostics. It does not own product command construction or semantic expectations.

Start with a small offline Cargo workspace that covers ordinary functions, two generic instances, a
local dependency, features, and a build-script tracked input. If another behavior requires a
different shape, add a separate fixture.

All environment-sensitive API calls run in isolated children. The parent test runner must not mutate
process-global environment variables. The [test strategy](test-strategy.md) defines the directory
layout, toolchain resolution, observations, and acceptance matrix.

Completion requires migrating existing process tests without losing their current assertions. A
helper with no real consumers does not satisfy this work.

## F3: Storage correctness

Implement the [architecture's durable bounds](mvp-architecture.md#durable-input-boundary) before
cache reuse starts reading candidate captures.

Cover each meaningful pre-commit failure boundary with deterministic tests. Use real temporary
files, malformed fixtures, and obstructed paths where practical. For an otherwise inaccessible
failure, a narrow private test seam is acceptable. Do not add a filesystem trait or production
fault-injection configuration.

A test verifies the completed-capture namespace, not only the returned error. It also verifies that
previous captures remain readable. Cache publication later extends this suite at the pointer
installation and final-rename boundaries.

## Readability and simplification

Apply `$rust-style` during every implementation checkpoint and again to the complete diff.

If a split adds only navigation, keep the straight-line function. Each module owns a named concept.
If that concept has children, use a directory with `mod.rs`. Keep constants documented beside their
definitions.

If compatibility or fallback code serves no approved contract, remove it. Keep ordinary input
validation and resource bounds required for durable storage. They protect the main workflow.

Write current architecture and limitations on main as behavior becomes implemented. Keep future
plans on `planning`. Contributor instructions must explain this distinction.

## Foundation acceptance

The foundation requires passing CI on both hosts and isolated process fixtures. Oversized durable
input must fail before deserialization. Publication tests must verify that incomplete captures
remain outside the completed namespace.

This does not complete the MVP. Proceed to Checkpoint A in the [work ledger](mvp-plan.md).
