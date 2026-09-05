# Testing Cargo Optic

The tests use Cargo's test runner, small Rust fixtures, and real compiler processes. There is no
separate scenario language or test backend.

## Run the checks

The repository pins Rust 1.98.1. Install its required components before running the suite:

```console
rustup component add rustc-dev llvm-tools clippy rustfmt
bash scripts/check-format.sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
bash scripts/check-install.sh
```

The installation check needs registry access during package and dependency setup. It does not
publish packages. It verifies archives, installs outside the checkout, and starts runtime tests
with a fresh driver cache. Linux also checks an independent library consumer and runs the installed
CLI on this repository's `cargo-optic-records` crate.

CI runs workspace and installed-CLI tests on Linux and macOS. Formatting includes standalone
compiler sources, not just modules that Cargo discovers.

## Choose the test boundary

| Behavior | Test location |
| --- | --- |
| Identity parsing, record invariants, and deserialization. | Records unit tests. |
| Durable limits, artifact ranges, and publication failures. | Store unit tests. |
| Driver cache inputs, wire protocol, and bounded LLVM parsing. | Compiler unit tests. |
| Actual Cargo selection, freshness, compiler settings, and retained stages. | Compiler integration tests. |
| Search ordering, immutable references, availability, and exact alias resolution. | Evidence unit tests. |
| Capture policy, failure sequences, stored source, and LLVM. | API integration tests. |
| Cargo subcommand discovery, output bytes, diagnostics, and closed stdout. | CLI integration tests. |
| Archive completeness and use outside the repository. | Installation script and consumer. |

Put a regression at the smallest boundary that proves the behavior. Use a process test when the
claim concerns Cargo or rustc behavior. A fabricated Cargo artifact does not prove real freshness.

## Isolate process tests

The unpublished `cargo-optic-test-support` crate copies fixtures into temporary workspaces. Each
workspace has separate Cargo, target, build, temporary, and observation directories. Its child
commands use the actual installed toolchain with cleared environments and offline local dependencies.

Apply scenario-specific environment variables to the child command. Never mutate the parent test
runner's environment. API tests that need process configuration start a fresh test-executable child.

Keep observations outside the fixture source tree. Otherwise, a counter or log file can invalidate
Cargo's default build-script tracking and change the behavior under test.

Private compiler tests use a small self-contained fixture when archive tests cannot use the
unpublished helper. Do not expose product APIs or add feature switches solely to reach private code.

## Prove the claim

Cache tests count actual selected-target compilation and driver compilation separately. Warm reuse
must preserve the capture ID, completion time, and evidence files. Elapsed time is not a cache oracle.

Source tests compare whole-item bytes against compiler-normalized snapshots. LLVM stage tests use
the pinned compiler's expected output paths and compare optimized output with its earlier stage.
Exact indexing tests include quoted symbols, aliases, unrelated large input, and range boundaries.

Failure tests assert what remains valid after failure, not just that an error occurred. For example,
failed publication must preserve old evidence without making its analysis token eligible for reuse.

For changes to compiler settings or capture publication, run the full cache journey after the focused
test. Before merging a product change, run the complete suite and both-host CI on the final revision.
The detailed acceptance contracts and execution evidence live on `planning`.
