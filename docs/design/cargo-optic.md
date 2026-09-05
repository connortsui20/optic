# `cargo-optic`

This document describes the current user product. Read the [product overview](overview.md) for the
goals and feature model. The complete internal architecture is in [`PLAN.md`](PLAN.md).

## Product

`cargo-optic` records compiler evidence from real Cargo builds. It finds concrete Rust compiler
instances and shows the exact evidence that belongs to each instance.

The product provides the `cargo optic` command. Its library exposes the same workflows through the
typed `cargo_optic::Application` API.

`cargo-ir` is an unpublished internal library. It collects compiler evidence and indexes LLVM IR.
It has no command and owns no persistent state.

A future TUI can use the application API. The current prototype contains no TUI, daemon, shared
selection, or client context.

The prototype supports explicit read-only access to another workspace's `.optic` directory. It
does not add a global writable store.

## Problem

A Rust code-generation investigation starts with one real Cargo target. The investigator must then
find a concrete function across codegen units and LLVM stages.

A generic definition can produce many compiler instances. Another crate can create those
instances, and rustc can place them in several codegen units.

LLVM can remove, clone, merge, or rename a body. Display paths can also differ between rustc and
LLVM. Text matching cannot prove which body belongs to one compiler instance.

Cargo Optic uses an exact-version rustc driver to record the raw LLVM symbol for each instance. It
connects an instance to a body only through equal raw symbols.

## Main workflow

The main command accepts a Rust definition query and normal Cargo target options:

```console
cargo optic show my_crate::kernel -p my-crate --lib --release --source
```

The command captures or reuses evidence and then searches the selected target. When several
instances match, it prints one complete `show --instance` command for each candidate.

Optimized LLVM IR is the default result. The user can select these additional views:

- `--output llvm-pre-opt` selects LLVM IR before optimization.
- `--output remarks` captures or shows LLVM optimization remarks.
- `--source` adds the captured Rust item.

The plain-text interface is the default for people and agents. `--format jsonl` writes versioned
JSON Lines events for programs.

## Controlled workflow

Separate commands support clients that control each step:

```console
cargo optic capture -p my-crate --lib --release --format jsonl
cargo optic find --capture CAPTURE_ID my_crate::kernel --format jsonl
cargo optic show --instance INSTANCE_ID --format jsonl
cargo optic inspect --capture CAPTURE_ID --format jsonl
```

Opaque capture and instance IDs connect these commands. An instance ID identifies its capture, so
`show --instance` does not need a capture ID.

Lookup first tests exact definition paths, display names, and raw symbols. A fallback lookup uses
a case-sensitive literal substring and requires at least three Unicode characters.

The `find` command supports crate, definition, and LLVM-availability filters. It reports truncation
and preserves enough identity data to distinguish equal display names.

## Streaming contract

The application API emits typed events for capture, inspection, lookup, source, and compiler
output. The CLI renders these events as plain text or JSON Lines.

Source and LLVM text use chunks of at most 64 KiB. The application does not build one complete
source item or LLVM body in memory before it returns data.

In text mode, Cargo progress and compiler diagnostics use standard error. Source, compiler output,
and the final result use standard output as they arrive.

In JSON Lines mode, all events use standard output. Each line contains one complete JSON value
with a version, sequence number, command, event name, and typed data.

A successful JSON Lines stream ends with one `complete` event. An operational error ends with one
`error` event that keeps the parsed command name.

If the output consumer closes the stream during capture, Cargo Optic cancels the capture. On Unix,
it terminates and reaps Cargo and its compiler descendants.

## Compiler contract

Use `cargo optic` without a toolchain prefix. Cargo Optic uses the Cargo and rustc that the
workspace selects through normal Rust configuration.

The selected rustc can use a stable, beta, or nightly release. It requires matching `rustc-dev` and
`llvm-tools` components.

Cargo Optic authorizes internal unstable access only for Cargo configuration discovery, driver
compilation, and selected-target compilation. It does not add unstable access to dependencies,
build scripts, or compiler probes.

The user does not set `RUSTC_BOOTSTRAP`. `inspect` reports the exact compiler identity and the
authorized unstable-access scopes.

Cargo Optic disables existing global and workspace compiler wrappers with a warning. The selected
target receives the evidence arguments and exact-version driver.

## Capture fidelity

The default faithful profile preserves the selected target's code-generation configuration. It
adds only arguments that save compiler evidence.

The enriched profile adds v0 symbol names and line-table debug information. The experiment profile
accepts explicit rustc arguments for a named experiment.

Cargo Optic uses `cargo rustc`, the normal target directory, and the normal dependency graph. Cargo
can reuse dependency artifacts from normal builds.

The selected target uses a separate analysis fingerprint. Cargo validates freshness before Cargo
Optic reuses a completed capture.

## Evidence

The prototype supports these evidence channels:

- Optimized LLVM IR.
- LLVM IR before optimization.
- Structured LLVM optimization remarks.
- Compiler instances, definitions, raw symbols, and codegen-unit placements.
- LLVM bodies, declarations, and aliases.
- Exact Rust source items from validated local snapshots.
- Compiler commands, build arguments, and compiler provenance.

The prototype does not support MIR, assembly, object files, native symbol sizes, linked symbols, or
inline occurrence navigation.

The store keeps large LLVM modules as files. Each body record contains a byte range, so one query
does not load the complete module.

## Remarks

A build-based `show --output remarks` request captures remarks automatically. A separate capture
uses `capture --remarks`.

The store distinguishes remarks that were not captured, a captured result with no records, and a
captured result with records. One instance or filter can still have no matching records.

Remark queries support kind, pass-name, and result-limit filters. An enriched capture provides
LLVM-emitted source locations.

## Comparison

`compare` accepts two exact instance IDs. It reports compatible and incompatible compiler or Cargo
dimensions before the structural LLVM delta.

The compatibility result includes the compiler commit, host, environment, and effective rustc
arguments. It ignores Optic arguments that only collect evidence.

The structural summary includes body bytes, instruction-like lines, vector lines, safety-check
symbols, and typed call categories. It separates runtime calls from compiler intrinsics.

These counts describe LLVM structure. They do not estimate cycles or replace a benchmark.

### Foreign comparison

`compare` accepts an optional `.optic` path for each instance. This form can compare two
worktrees without copying their captures into one store.

The application opens each foreign catalog in read-only mode. The current structural comparison
continues to operate on typed summaries.

## Persistent store

Each Cargo workspace stores evidence below `.optic/store`. The store contains a SQLite catalog,
content-addressed blobs, pending evidence, and private work directories.

Locks remain below `.optic/locks`. They coordinate schema access, commands, capture writers, and
evidence removal.

Completed captures are immutable. The prototype has no current capture, labels, pins, automatic
eviction, or client session. Its soft storage policy checks capture work without reserving a hard
retained-state budget.

`remove` deletes one catalog capture. `gc` removes unreferenced blobs. `verify` validates referenced
blob digests.

`clean` removes `.optic/store` only. It preserves `.optic/locks` and durable configuration below
`.optic`.

Schema 10 stores every blob as one zstd frame and reports physical storage accounting. The
prototype also provides pending list, inspection, and targeted removal commands.

Read [federated evidence and storage admission](federated-storage.md) for these contracts and their
MVP limits.

## Recovery

After an ingestion failure, Cargo Optic retains validated post-compilation evidence. The next
matching request asks Cargo to validate freshness.

When the evidence is fresh, Cargo Optic resumes ingestion without another selected-target
compilation. When tracked inputs changed, it removes the stale evidence and recompiles.

No partial capture becomes visible. Malformed pending data remains available for diagnosis and
causes a clear error.

## Concurrency

SQLite uses write-ahead logging. Read commands can query completed captures while another command
prepares evidence.

Filesystem locks coordinate clean, store creation, publication, removal, and garbage collection.
One short SQLite transaction publishes a completed capture.

Evidence ingestion writes records to a private staging catalog as it reads them. Publication joins
instances to bodies, declarations, and aliases only within the same capture.

The `.optic` directory is a workspace store, not a session. Every read command names a capture or
instance explicitly.

## Product versions

The current prototype uses store schema 10 and JSON Lines transport 1. It also uses evidence request
5, pending marker 1, and compiler manifest protocol 3.

These prototype formats have no compatibility promise. The current version does not detect or
migrate older formats and can return an error or panic when it reads older state.

## Non-goals

- Profiling.
- Benchmark orchestration.
- A general LLVM pass debugger.
- A compiler database for unrelated tools.
- IDE integration.
- A web interface.
- A TUI in the current prototype.

The [`cargo-ir` spike results](cargo-ir.md) preserve the original compiler research. The later
[`research`](../research/README.md) provides detailed historical evidence.

## License

The project is available under the [Apache License 2.0](../../LICENSE-APACHE) or the
[MIT license](../../LICENSE-MIT), at your option.
