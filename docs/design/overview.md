# Cargo Optic product overview

Cargo Optic connects one concrete Rust compiler instance to evidence from one real Cargo build.
This document explains the goals and feature boundaries of the current prototype.

The replacement architecture has three design documents:

- [Future architecture](future-architecture.md) defines the long-term product boundaries.
- [MVP architecture](mvp-architecture.md) defines the first complete package shape.
- [MVP plan](mvp-plan.md) defines the first vertical slice and its user-visible pull requests.

Use these documents for more detail:

- [Capture and evidence](capture.md) explains what the tool records and why the evidence is valid.
- [Query and comparison](query.md) explains lookup, output selection, remarks, and comparison.
- [Persistent storage](storage.md) explains reuse, recovery, concurrency, and lifecycle commands.
- [Federated evidence](federated-storage.md) defines explicit foreign reads and storage admission.
- [Product design](cargo-optic.md) gives the detailed command contracts.
- [Architecture](PLAN.md) gives the internal component and data-flow design.

## The main question

Cargo Optic answers this question:

> What did the selected compiler produce for this concrete instance in this Cargo build?

The answer can include Rust source, LLVM IR, optimization remarks, and exact build provenance.

## Why the tool exists

Rust source names do not map directly to compiler output. A generic function can produce many
concrete instances. Another crate can create an instance, and rustc can place it in several codegen
units.

LLVM adds more uncertainty. It can inline, remove, clone, merge, or rename a function body. A
similar display name does not prove that two compiler records describe the same function.

Existing tools can show compiler artifacts. The investigator must still connect each artifact to
the correct function, build, and compiler stage.

Cargo Optic records these relationships during the build. It stores the resulting evidence for
later queries and comparisons.

## Product goals

Cargo Optic has these high-level goals:

1. Use the Cargo build that the project already defines.
2. Identify concrete compiler instances without display-name guesses.
3. Show useful compiler evidence for one exact instance.
4. Record the build and compiler facts that affect the evidence.
5. Reuse expensive evidence only after Cargo reports that the build is fresh.
6. Keep completed evidence immutable and available for later queries.
7. Support people, agents, and programs through the same product boundary.
8. Report ambiguity or missing evidence instead of inventing a relationship.
9. Read completed evidence from another explicit workspace store without durable Optic changes.
10. Apply a soft storage-admission policy and report physical retained size.

The product focuses on compiler-output investigations. It is not a profiler, benchmark runner,
build system, or general compiler debugger.

## The core model

The following terms describe the product:

| Term | Meaning |
| --- | --- |
| Build request | One Cargo target, profile, feature set, target triple, and evidence profile. |
| Capture | Immutable evidence from one completed request and compiler configuration. |
| Definition | One source-level Rust item, such as a generic function. |
| Instance | One concrete compiler form of a definition, such as `kernel::<u64>`. |
| Placement | One codegen-unit location where rustc placed an instance. |
| Module | One saved LLVM artifact from one compiler stage. |
| Body | One standalone LLVM function with an exact byte range in a module. |
| Remark | One structured report from an LLVM optimization pass. |

A definition can have many instances. An instance can have many placements and stage records. A
stage can contain a definition, declaration, alias, or no exact symbol for the instance.

Cargo Optic keeps these facts separate. It does not turn a similar name into an evidence
relationship.

This model has several visible consequences:

- A generic query can return several concrete instances.
- A pre-optimization body can exist without an optimized body.
- Rustc instance records that use the same raw symbol remain separate.
- An instance ID identifies one instance in one capture.
- The prototype does not create one logical function ID across captures.

## Main workflow

The main command combines capture, lookup, and display:

```console
cargo optic show my_crate::kernel -p my-crate --lib --release --source
```

This command follows five product steps:

1. Cargo Optic resolves one Cargo workspace, package, and target.
2. It captures new evidence or reuses a fresh capture.
3. It finds concrete instances that match the query.
4. If one instance matches, it shows the selected compiler output.
5. If several instances match, it prints complete `show --instance` commands.

Optimized LLVM IR is the default output. The `--source` option adds the captured Rust item.

The user can select these other outputs:

- `--output llvm-pre-opt` shows LLVM IR before the LLVM optimization pipeline.
- `--output remarks` shows structured LLVM optimization remarks.

## Feature groups

The current prototype implements these product features:

| Feature | Result |
| --- | --- |
| Capture | Records evidence for one selected Cargo target. |
| Reuse | Reuses a completed capture after Cargo accepts its freshness. |
| Lookup | Finds exact or substring matches for concrete instances. |
| Source | Shows the captured Rust item for an exact local definition span. |
| LLVM | Shows optimized or pre-optimization LLVM for one instance. |
| Remarks | Shows structured LLVM optimization records with filters. |
| Inspection | Shows the compiler, request, arguments, and artifacts. |
| Comparison | Compares compatibility and LLVM structure for two instances. |
| Storage | Keeps immutable captures and content-addressed evidence in the workspace. |
| Recovery | Resumes valid evidence ingestion after an ingestion error. |
| Automation | Provides versioned JSON Lines events and a typed streaming application API. |
| Foreign reads | Reads and compares evidence from explicit workspace stores. |
| Storage policy | Reports physical retained size and rejects capture work at policy checkpoints. |

The prototype keeps capture and lifecycle mutations local to one workspace. Read
[federated evidence and storage admission](federated-storage.md) for the foreign-read contract and
storage-policy limits.

## Controlled workflow

Separate commands expose each product step:

| Command | Purpose |
| --- | --- |
| `capture` | Captures evidence or reuses a fresh capture. |
| `list-captures` | Lists completed captures in the workspace. |
| `find` | Finds concrete instances in one capture. |
| `show` | Shows one compiler output for one instance. |
| `inspect` | Shows the request, compiler, arguments, and artifacts. |
| `compare` | Compares compatibility and LLVM structure for two exact instances. |
| `status` | Shows store size and object counts. |
| `pending` | Lists, inspects, and removes retained compiler runs. |
| `verify` | Validates the digest of each referenced blob. |
| `remove` | Removes one capture from the catalog. |
| `gc` | Removes blobs that no completed capture references. |
| `clean` | Removes all stored Optic evidence from the workspace. |

Plain text is the default output. The `--format jsonl` option writes versioned JSON Lines events
for programs.

The stream contains typed progress, diagnostic, data, and terminal events. Source and LLVM text
use chunks of at most 64 KiB. A successful stream ends with one `complete` event.

Each read command uses a capture ID or instance ID. The workspace has no current capture, shared
selection, or client session.

## What the prototype can answer

The current prototype can answer these questions:

- Which concrete instances did rustc collect for this target?
- Which optimized or pre-optimization LLVM bodies exist for one instance?
- What exact Rust item produced the instance?
- Which LLVM declarations or aliases use the same raw symbol?
- Which optimization remarks refer to the instance?
- Which compiler, configuration, and arguments produced the evidence?
- Did selected LLVM structure change between two exact instances?
- Can a completed capture be reused without another selected-target compilation?

These answers give agents a typed and versioned interface instead of an ad hoc collection of
artifact paths.

## Current limits

The prototype has these product limits:

- It does not capture MIR, assembly, object files, native symbol sizes, or linked-product symbols.
- It does not attribute inlined instructions to enclosing optimized bodies.
- It does not follow LLVM clone, merge, or rename lineage.
- It does not create stable logical function identities across captures.
- It does not profile code or measure runtime performance.
- It does not orchestrate benchmarks.
- It does not install the required rustc components.
- It does not support labels, pins, hard retention budgets, or automatic eviction.
- It does not contain a TUI, daemon, web interface, or IDE integration.
- It supports the default `rustc` on Unix hosts.
- It disables configured compiler wrappers with a warning.
- It does not support custom compiler commands, Windows hosts, or rustc response files.

The current end-to-end acceptance test runs on Apple silicon macOS. Other hosts and cross-target
builds need separate contract tests.

## Product boundary

`cargo-optic` is the only user product. Its binary provides `cargo optic`, and its library provides
the typed `cargo_optic::Application` API.

`cargo-ir` is an unpublished internal library. It owns Cargo execution, rustc integration, LLVM
artifact discovery, LLVM indexing, and optimization remark parsing.

The exact-version rustc driver is an internal helper. Rustc-private types do not cross its
versioned protocol boundary.

A future TUI can use the application API. The prototype does not require another product command
or another public crate.

## Prototype status

The current prototype uses these internal format versions:

- The store schema is version 10.
- The JSON Lines transport is version 1.
- The evidence request is version 5.
- The pending marker is version 1.
- The compiler identity protocol is version 3.

These formats do not have a compatibility promise. The current version does not detect or migrate
older formats and can return an error or panic when it reads older state.
