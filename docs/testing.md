# Testing Cargo Optic

The tests use Cargo's test runner, small Rust fixtures, and real compiler processes.

## Run the checks

The repository uses the compiler and components in [rust-toolchain.toml](../rust-toolchain.toml).
The formatting check also requires Python 3.
Run these commands from the repository root:

```console
rustup component add rustc-dev llvm-tools clippy rustfmt
bash scripts/check-format.sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
```

Formatting includes the standalone driver and fixtures that `cargo fmt` does not discover.
The check rejects Rust lines over 100 columns, except exact compiler-source permalinks in
documentation.
CI runs workspace tests on Linux and macOS.

## Check the packages

After committing the changes, run the installation check from a clean checkout:

```console
bash scripts/check-install.sh
```

The script requires Python 3.11+, Git, and registry access during package and dependency setup.
It rejects uncommitted changes and does not publish packages.

The script checks archives, installs outside the checkout, and starts runtime tests with a fresh
driver cache. The Linux checks also cover an independent library consumer and a capture of the
committed `cargo-optic-records` source.

The script prints its temporary evidence directory and retains it for diagnosis. The repository
snapshot uses `git archive`. Its capture does not inherit ancestor Cargo configuration or write into
the checkout's store.

CI runs the installed CLI checks on Linux and macOS.

When the compiler source layout changes, preserve every embedded source needed to build the driver.
Keep the unpublished test helper as a path-only dev-dependency without a version.
Exclude integration tests that require the helper from package archives.
The script checks that the archives work without the helper or repository-relative files.

## Choose the test boundary

| Behavior | Test location |
| --- | --- |
| Identity parsing, record invariants, and deserialization. | Records unit tests. |
| Durable limits, artifact ranges, and publication failures. | Store unit tests. |
| Driver cache inputs, wire protocol, and bounded LLVM parsing. | Compiler unit tests. |
| Cargo selection, freshness, configuration, and retained stages. | Compiler integration tests. |
| Search, immutable references, availability, and exact aliases. | Evidence unit tests. |
| Capture policy, failure sequences, stored source, and LLVM. | API integration tests. |
| Subcommand discovery, output, diagnostics, and closed stdout. | CLI integration tests. |
| Archive completeness and use outside the repository. | Installation script and consumer. |

Put a regression at the smallest boundary that proves the behavior.
For Cargo or rustc behavior, use a process test. A fabricated Cargo artifact does not prove real
freshness.

Use semantic assertions for command behavior. Use exact bytes for a source range or LLVM excerpt.
Avoid whole-output snapshots of compiler diagnostics or full LLVM modules.

## Isolate process tests

The unpublished `cargo-optic-test-support` crate copies fixtures into temporary workspaces. Each
workspace has separate Cargo, target, build, temporary, and observation directories. Its child
commands use the installed toolchain with cleared environments and offline local dependencies.

Apply scenario-specific environment variables to the child command. Never mutate the parent test
runner's environment. API tests that need process configuration start a fresh test-executable child.

Keep observations outside the fixture source tree. Otherwise, a counter or log file can invalidate
Cargo's default build-script tracking and change the behavior under test.

When archive tests cannot use the unpublished helper, use a small self-contained compiler fixture.
Do not expose product APIs or add feature switches solely to reach private code.

## Prove the claim

Cache tests count actual selected-target compilation and driver compilation separately. Each
observation test must first establish a positive cold count. Zero-only assertions can pass with
broken instrumentation. Warm reuse must preserve the capture ID, completion time, and evidence
files. Elapsed time does not prove cache reuse.

Source tests compare whole-item bytes against compiler-normalized snapshots. LLVM stage tests use
the pinned compiler's expected output paths and compare optimized output with its earlier stage.
Exact indexing tests include quoted symbols, aliases, unrelated large input, and range boundaries.

Failure tests assert what remains valid after failure, not just that an error occurred. For example,
failed publication must preserve old evidence without making its analysis token eligible for reuse.

For changes to compiler configuration or capture publication, run the full cache journey after the
focused test. Before merging a product change, require the complete suite and both-host CI to pass
on the final revision.
