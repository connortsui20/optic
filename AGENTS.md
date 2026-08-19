# Agent guide

Read [README.md](README.md) before you change the product behavior. Use the
[fixture guide](crates/cargo-optic/tests/fixtures/generic/example.md) for a manual test.

## Product boundary

- `cargo-optic` is the only user product.
- `cargo-ir` is an unpublished library for the compiler and LLVM boundary.
- Do not add a separate `cargo-ir` command or publish either crate.
- A future TUI can use the `cargo-optic::Application` API. This prototype does not contain a TUI.
- Do not widen the product scope unless the user requests that change.

## Current workflow

- `cargo optic show QUERY [BUILD OPTIONS]` is the main command.
- This command captures or reuses evidence, finds an instance, and shows one compiler output.
- Optimized LLVM IR is the default output.
- The `--source` option adds the captured Rust item.
- The `--output llvm-pre-opt` option selects LLVM IR from before optimization.
- Plain text is the default format for humans and agents.
- The `--format json` option provides a versioned transport format for programs.
- An ambiguous query prints complete `show --instance` commands.

## Persistent state

- `.optic` contains immutable captures, a SQLite catalog, blobs, and temporary staging files.
- `.optic.lock` coordinates store operations and remains after `cargo optic clean`.
- The clean command removes `.optic` only. It keeps the Cargo target directory.
- There is no current capture and no persistent client session.
- Each read command uses a capture ID or an instance ID.
- An instance ID identifies its capture, so `show --instance` does not need a capture ID.
- New IDs contain random UUIDv4 suffixes.
- Text output shows at least 12 hexadecimal characters and highlights the shortest unique prefix.
- The schema version is 4. The store migrates schema versions 1, 2, and 3.

Capture writers use a file lock. Read commands can use completed captures in parallel. An operation
lock prevents `clean` from removing a store that another process uses.

## Cargo behavior

`cargo-ir` uses `cargo rustc` with the normal target directory and dependency graph. Normal Cargo
commands can reuse dependency artifacts from an Optic build.

The selected target uses analysis flags and `-Z no-link`. It has a separate Cargo fingerprint. A
normal Cargo command can compile or link that target after an Optic capture.

The evidence does not exactly match a normal build. Optic enables saved temporary files, v0 symbol
names, and line-table debug information. An exact-version rustc driver records compiler identities.
The driver runs only for the selected target.

## Code map

- `crates/cargo-optic/src/app.rs` coordinates capture, cache reuse, lookup, and source display.
- `crates/cargo-optic/src/cli.rs` owns command validation and text or JSON output.
- `crates/cargo-optic/src/store.rs` owns SQLite, IDs, locks, migrations, and blobs.
- `crates/cargo-optic/src/source.rs` snapshots build inputs and finds source items.
- `crates/cargo-ir/src/capture.rs` runs Cargo and collects LLVM evidence.
- `crates/cargo-ir/src/llvm.rs` indexes LLVM function bodies by byte range.
- `crates/cargo-optic/tests/e2e.rs` covers the complete supported workflow.

## Change rules

- Apply the `rust-style` skill to every Rust change when that skill is available.
- Preserve the typed application boundary. Do not parse CLI output inside the product.
- Keep large LLVM modules on disk. Store byte ranges instead of parsed modules.
- Treat IDs as opaque values. Do not expose SQLite row IDs or artifact paths.
- Return an error for invalid user or stored data. Reserve panics for internal invariants.
- Preserve the existing plain-text and versioned JSON contracts unless the user requests a break.

## Validation

Run these commands after each Rust change:

```console
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
git diff --check
```

The end-to-end test requires nightly rustc and its matching `llvm-tools` and `rustc-dev` components.

## Known limits

- The prototype supports optimized and pre-optimization LLVM IR only.
- It does not support MIR, assembly, object files, LTO stages, or a TUI.
- Source lookup uses syntax and path scoring. It omits an ambiguous source result.
- The cache cannot find every external input that a build script reads.
- The current acceptance test covers Apple silicon macOS.
