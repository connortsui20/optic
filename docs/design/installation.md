# Packaged MVP verification

The MVP is an installable release candidate, not merely a binary that works from this checkout.
Registry publication is a separate authorized release action.

## Package contents

Keep product packages on one initial `0.1.x` version with complete description, repository, license,
and readme metadata. Keep workspace product dependencies as path plus matching registry version.

During release-candidate preparation, remove `publish = false` from the seven product packages. The
test-support package remains unpublished.

Embed every source file needed to build the standalone driver. Inspect the package file list after
module restructuring, since a missing included file can break runtime collection after installation.

Use the existing Cargo packaging mechanism:

```console
cargo package --workspace --exclude cargo-optic-test-support --locked
```

Keep verification enabled. Native workspace packaging handles interdependent package selection. Do
not add a custom local registry or use `--no-verify` as the passing release check. See
[cargo package](https://doc.rust-lang.org/cargo/commands/cargo-package.html).

## Development-only tests

Reference the unpublished test-support crate only through path-only dev-dependencies without a
version. Cargo removes those dependencies from normalized package manifests. See
[development dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#development-dependencies).

Exclude integration test files that depend on the helper from package archives. Keep self-contained
unit tests and required driver assets. Verify that no normalized dependency or retained test
requires the unpublished helper.

Do not move integration tests into normal product dependencies, publish a test framework, or remove
repository tests to make packaging pass.

## Installed CLI journey

Package the workspace, then extract the generated archives into a temporary root outside the
repository. Run the verification from a directory with no access through relative repository paths.

Before the packages exist in the registry, use temporary Cargo patches pointing only to the
extracted sibling packages. Apply those patches in the verification environment, not in published
manifests. Do not patch any dependency to the original workspace.

Install the extracted CLI package into a separate install prefix. Inspect dependency resolution to
verify that all Optic source comes from the archives. Installation can fetch declared third-party
dependencies as a distinct setup step.

Use a separate fresh runtime Cargo home and fixture target directory. Preserve the installed rustup
components, as specified in the [test strategy](test-strategy.md#process-isolation).

Run real `cargo optic` discovery and the documented release-profile journey:

1. Capture the fixture and verify its concrete instances.
2. List and find the completed capture.
3. Repeat capture and verify the same ID and completion time.
4. Show local source and exact LLVM through references printed by find.
5. Change source and verify a new capture.
6. Read old evidence and verify it still contains the old bytes.
7. Force fresh capture while retaining compatible driver and dependency reuse.

The runtime must not depend on the build workspace, repository fixtures, or an already provisioned
Optic driver. The first capture compiles the driver from packaged embedded sources.

Include a Git-backed fixture with default build-script tracking and no preexisting Optic ignore rule.
Its repeated capture verifies that store initialization does not make its own evidence stale.

Test missing required components through focused compiler tests. Do not uninstall the developer's
toolchain to produce an installation error.

## External library consumer

Build a small independent Cargo package using the packaged `cargo-optic-api` dependency and its
`optic` crate name. Use only exported APIs, including capture policy/outcome, search references,
typed availability, and writer-based evidence output.

Run the same central workflow from that consumer. It must not depend on private subsystem modules,
the unpublished helper, or repository-relative assets.

Temporary extracted-sibling patches prove prepublication archive completeness and consumer APIs.
They do not prove that a registry publication succeeded. After a separately authorized publication,
repeat an unpatched `cargo install cargo-optic --locked` smoke test.

## CI and completion

Run the release-candidate package and installed workflow check in the final CI gate. Execute the
installed CLI journey on both supported hosts. The external-library compilation can run on Linux.

Add one Linux self-hosting smoke test using the installed CLI against this repository's
`cargo-optic-records` library in release mode. Capture, find a known concrete function, show its
source and exact LLVM, then verify warm reuse. Use a separate Cargo target directory. This checks
real product source and its dependency graph; it does not replace the focused fixture or archive
isolation tests. The self-hosting test can use the checkout explicitly, unlike the installed fixture
and external-consumer tests.

Keep the script small and direct. Do not add a release orchestrator or automated registry
publishing.

Record archive versions, exact compiler/LLVM identity, supported hosts, installed command output,
and final CI revision. Document that artifacts are prototype formats without migration guarantees.
