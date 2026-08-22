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
- The `--output remarks` option captures or shows LLVM optimization remarks.
- Plain text is the default format for humans and agents.
- The `--format jsonl` option provides versioned JSON Lines events for programs.
- An ambiguous query prints complete `show --instance` commands.

## Persistent state

- `.optic/store` contains immutable captures, the SQLite catalog, blobs, and temporary staging files.
- `.optic/locks` coordinates store operations and remains after `cargo optic clean`.
- The clean command removes `.optic/store` only. It keeps the Cargo target directory.
- Keep other entries below `.optic`, including `.optic/config.toml`.
- Cargo Optic does not create a `.optic.lock` file in the workspace root.
- There is no current capture and no persistent client session.
- Each read command uses a capture ID or an instance ID.
- An instance ID identifies its capture, so `show --instance` does not need a capture ID.
- New IDs contain random UUIDv4 suffixes.
- Text output shows at least 12 hexadecimal characters and highlights the shortest unique prefix.
- The schema version is 10. Older stores require `cargo optic clean`.
- Schema 10 stores content-addressed blobs as zstd level-3 frames. Blob IDs hash the logical,
  uncompressed bytes.
- Failed post-compilation ingestion can leave validated evidence below `.optic/store/pending`.
- A matching request validates Cargo freshness before it resumes retained ingestion.
- `cargo optic pending` lists retained runs. Its `inspect` and `remove` subcommands select opaque
  pending IDs.

`--optic-dir PATH` opens an existing foreign `.optic` store for read-only commands. A comparison
can select its before and after stores independently.

Capture writers use a file lock. Read commands can use completed captures in parallel. An operation
lock prevents `clean` from removing a store that another process uses.

## Cargo behavior

`cargo-ir` uses `cargo rustc` with the normal target directory and dependency graph. Normal Cargo
commands can reuse dependency artifacts from an Optic build.

Use `cargo optic` without a toolchain prefix. Cargo Optic uses the Cargo and rustc that the
workspace selects through the normal Rust configuration.

The selected rustc requires matching `rustc-dev` and `llvm-tools` components. Cargo Optic enables
unstable access only for Cargo configuration discovery, exact-version driver compilation, and
selected-target compilation. It does not require a user-supplied `RUSTC_BOOTSTRAP` value.

Cargo Optic does not enable unstable access for dependencies, build scripts, or compiler probes.

Text-mode Cargo progress and compiler diagnostics stream to standard error. Text results use
standard output. JSON Lines events use standard output. One Cargo JSON message and the retained
failure tail each have a 1 MiB limit.

If a capture consumer stops, Cargo Optic terminates and reaps Cargo and its compiler descendants.

The selected target uses saved-temporary arguments and has a separate Cargo fingerprint. Cargo
checks this fingerprint before Optic reuses evidence.

The faithful profile preserves target code-generation settings. The enriched profile adds v0
symbols and line tables. The experiment profile adds explicit user arguments. An exact-version
rustc driver records compiler identities for the selected target.

## Code map

- `crates/cargo-optic/src/app.rs` coordinates capture, cache reuse, lookup, and source display.
- `crates/cargo-optic/src/cli.rs` owns command validation and text or JSON Lines output.
- `crates/cargo-optic/src/store.rs` owns SQLite, IDs, locks, lifecycle operations, and blobs.
- `crates/cargo-optic/src/source.rs` snapshots build inputs and finds source items.
- `crates/cargo-optic/src/pending.rs` validates recoverable post-compilation evidence.
- `crates/cargo-ir/src/capture.rs` runs Cargo and collects LLVM evidence.
- `crates/cargo-ir/src/cargo_output.rs` streams bounded Cargo output and tracks artifact freshness.
- `crates/cargo-ir/src/llvm.rs` indexes LLVM function bodies by byte range.
- `crates/cargo-ir/src/remarks.rs` parses bounded LLVM optimization remarks.
- `crates/cargo-optic/tests/e2e.rs` covers the complete supported workflow.

## Change rules

- Apply the `rust-style` skill to every Rust change when that skill is available.
- Preserve the typed application boundary. Do not parse CLI output inside the product.
- Keep large LLVM modules on disk. Store byte ranges instead of parsed modules.
- Treat IDs as opaque values. Do not expose SQLite row IDs or artifact paths.
- Return an error for invalid user or stored data. Reserve panics for internal invariants.
- Preserve the existing plain-text and versioned JSON Lines contracts unless the user requests a
  break.

## Validation

Run these commands after each Rust change:

```console
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
git diff --check
```

The end-to-end test requires the matching `llvm-tools` and `rustc-dev` components for the selected
workspace rustc.

## Known limits

- The prototype supports optimized LLVM IR, pre-optimization LLVM IR, and optimization remarks.
- It does not support MIR, assembly, object files, or a TUI.
- Source lookup requires an exact rustc definition span and canonical local source path.
- The rustc identity driver returns an error for non-UTF-8 compiler arguments.
- Cargo cannot find undeclared build-script inputs.
- Inline occurrences do not link to enclosing optimized bodies.
- The current acceptance test covers Apple silicon macOS.
