# Implementation reference

This document contains design details for the collector, catalog, and artifact store. If an
implementation decision depends on a detail, read the applicable section.

This document preserves a broad pre-prototype design. Names such as `CollectionRecord`, client
contexts, retention tiers, and `.optic/staging` do not describe the current implementation.

Read the [`design plan`](../../design/PLAN.md) for the current store, recovery, lookup, and compiler
architecture.

## Data flow

```text
collection request
       |
       +-- preflight and source baseline
       |
       v
Cargo JSON stream --------------------------------------+
       |                                                |
       v                                                |
outer rustc wrapper                                     |
       |                                                |
       +-- probe or dependency -> original wrappers -> rustc
       |                                                |
       +-- selected target -> original wrappers          |
                                  |                     |
                                  v                     |
                           optic-rustc-driver            |
                              |           |              |
                              v           v              v
                    identity manifest   rustc      isolated outputs
                              |           |              |
                              +-----------+--------------+
                                          |
                                          v
                                coordinator and importer
                                   |              |
                                   v              v
                             SQLite catalog   immutable blobs
```

The wrapper writes captured files to a temporary area. It publishes one complete invocation
manifest with an atomic rename. It does not open SQLite. Only the coordinator writes to the SQLite
catalog.

## Client model

The `.optic/` directory is one shared project store. It is not a session. A stateless request names
the collection, instance, body, or comparison that it uses.

The product CLI and future TUI use the `cargo-optic` application library. The internal `cargo-ir`
library has no CLI or persistent identifiers. All product clients use the same opaque identifiers.

Machine-readable commands return versioned JSON. Long operations return versioned JSON Lines
events. Standard output contains results. Standard error contains diagnostics. Errors contain a
stable code and the evidence that caused the error.

An optional `ClientContext` stores navigation state for one client. Each TUI, shell, or agent uses a
different context ID. Each context update supplies an expected revision. A revision conflict does
not overwrite the newer state.

Stateless commands do not read or change a client context. Read queries do not update access times
unless the caller requests that mutation.

A mutating request can include an idempotency key. A repeated key returns the original operation.
The key does not merge independent collection requests that have equal arguments.

## Logical records

### Collection records

`CollectionRecord` describes one requested Cargo command. It contains:

- An opaque collection ID and an optional idempotency key.
- Cargo arguments, current directory, toolchain, targets, profile, and features.
- The requested target and environment overrides.
- Compile-only or compile-and-run policy.
- Cargo metadata and unit graph, when available.
- Source baseline, worktree state, timing, and exit status.
- Cargo events, diagnostics, warnings, and unmatched activity.
- References to new or reused compiler evidence.

`PlannedUnit` describes one unit from Cargo's unstable unit graph. It is planning evidence, not
proof of a rustc invocation.

`CargoArtifactObservation` stores one Cargo artifact event. It includes freshness, filenames, crate
types, executable path, profile, and target.

`BuildScriptObservation` stores one sanitized build-script event. Cargo can replay this event from
its cache, so the event does not prove execution.

### Compiler records

`CompilerInvocation` describes one actual process. It contains:

- Exact argument bytes and a readable argument form.
- Current directory and an approved environment subset.
- Compiler path, wrapper chain, rustc commit, LLVM version, host, and target.
- Source, crate name, crate types, output directory, metadata hashes, and output configuration.
- Requested and effective code-generation configuration.
- Captured response files and standard input.
- Exit status, signal, timing, warnings, and injected flags.

Keep exact arguments after structured parsing. Rustc accepts repeated flags, joined and split
forms, response files, and custom target paths.

For each derived configuration value, record how Optic obtained it. Use `requested`,
`compiler-default`, `inferred`, or `observed`.

`CompilationEvidence` groups immutable outputs from one successful rustc invocation. A failed
invocation remains diagnostic data and never becomes reusable evidence.

### Artifact records

`Artifact` describes one file or stream. It stores the content digest, length, media type,
provenance, and capture method.

`ModuleStage` describes one compiler stage. It stores:

- Backend.
- Stage group and versioned stage name.
- CGU or partition identity.
- LTO scope.
- Capture fidelity.
- Evidence for every inferred field.

Artifacts with unknown stage suffixes remain available for queries.

`ModuleIndex` stores 64-bit byte offsets for:

- Function definitions and declarations.
- Aliases, globals, and indirect functions that select an implementation at load time.
- Named types and attribute groups.
- Metadata nodes and named metadata.

Function records also keep linkage, visibility, calling convention, section, COMDAT, attributes,
and target features. A COMDAT group tells the linker that it can keep one equivalent definition.
The loader resolves module context and debug metadata on demand.

### Identity records

`DefinitionOrigin` contains the defining package, crate target, Rust path, and optional source span.

`CompilerInstance` contains the source definition path, concrete display name, raw symbol, generic
arguments, and producer unit. The adapter does not use display text for an exact relationship.

`MonoPlacement` connects one instance to every reported CGU and linkage.

`PhysicalSymbol` belongs to one module stage. It contains the raw name, mangling scheme, readable
name, linkage, visibility, generated LLVM identifier (GUID), and native size when known.

Store each identity fact separately:

```text
InstanceEvidence
  collected facts[]
  bodies[]
  aliases[]
  declarations[]
  inline occurrences[]
  object symbols[]
  unresolved relationships[]
```

Every relationship stores a confidence value:

- `compiler_exact` uses a compiler-owned key or mono record.
- `symbol_exact` uses an exact raw symbol or alias inside one build.
- `debug_supported` uses debug linkage and scope metadata.
- `structural` uses matching origin, path, arguments, compiler, target, and configuration.
- `ambiguous` retains several candidates.
- `unmatched` retains evidence without a candidate.

Readable text can rank lookup candidates. It cannot create a relationship between evidence records.

## Collection protocol

### Preflight

Create a private collection directory below `.optic/staging/`. Write the request manifest before
Cargo starts.

Record:

- `rustc -vV`.
- `rustc --print sysroot`.
- The target specification and active `cfg` values.
- Cargo metadata and the unit graph.
- A source baseline for workspace and resolved package files.

Use Cargo's normal target directory and dependency graph. Record the effective target and build
directories. Never clean the Cargo target directory.

### Wrapper composition

Resolve both wrapper values from the environment and Cargo configuration. An empty environment value
disables its wrapper. Install Optic as the outer `RUSTC_WRAPPER`.

Cargo normally starts:

```text
global-wrapper workspace-wrapper rustc <arguments>
```

After installation, Cargo starts the Optic wrapper:

```text
optic-wrapper workspace-wrapper rustc <arguments>
```

For a probe or dependency, Optic reconstructs the original chain:

```text
original-global-wrapper workspace-wrapper rustc <arguments>
```

For the selected target, Optic inserts the driver in the compiler position:

```text
original-global-wrapper workspace-wrapper optic-rustc-driver rustc <arguments>
```

This order preserves the behavior and Cargo fingerprint of the existing workspace wrapper. Use a
wrapper-depth marker to prevent recursion. Pass compiler probes and dependencies through without
evidence flags.

### Rustc driver

Build the driver with the active compiler and its matching `rustc-dev` libraries. The driver has no
Cargo dependencies. This build compiles only the helper source. It does not compile the workspace or
its dependencies.

Cache the executable by host, rustc commit, and source digest. Validate its protocol before use.

Use a global cache below `$CARGO_HOME/optic/drivers`. Use a file lock and an atomic rename when two
collections request the same driver.

Run the driver only for the selected compiler invocation. Match that invocation through the private
`-Z temps-dir` value that Optic adds to the selected target.

In `after_analysis`, request rustc's monomorphization partitions. Record each function's definition
path, concrete display name, raw symbol, and CGU placements. Then continue the same compilation.

Write a versioned, length-prefixed manifest with these fields:

1. A fixed magic header and protocol version.
2. The rustc commit hash.
3. The number of compiler instances.
4. The definition path, display name, raw symbol, and CGU list for each instance.

Publish the manifest only after rustc succeeds. Reject a missing, truncated, oversized,
wrong-version, or wrong-toolchain manifest. Do not use a display-name fallback.

Join compiler instances to LLVM bodies only by raw-symbol equality. If LLVM removes or renames the
symbol, report no standalone body for that stage. Do not infer clone lineage from a suffix.

### Invocation classification

A compilation candidate normally contains a source input, `--crate-name`, `--out-dir`, and a
code-generation output. Unknown calls pass through and receive a warning.

Determine the target for each rustc invocation. If `--target` is absent, use the compiler host.

Detect a non-LLVM backend before adding LLVM flags. If scope classification is ambiguous, do not add
evidence flags.

### Compiler supervision

Create unique directories for outputs, remarks, mono statistics, and streams. Capture response files
before the child can remove them.

Read standard output and standard error concurrently. Forward original bytes without reordering
either stream. Do not require UTF-8.

Preserve the child exit code and signal behavior. On cancellation, relay the signal and leave an
incomplete manifest for later cleanup.

After a successful child exit:

1. Close both stream readers.
2. Enumerate invocation-owned output directories without following unexpected symbolic links.
3. Parse rustc artifact messages and dep-info.
4. Copy mutable outputs through a stable file descriptor.
5. Write and atomically rename the complete invocation manifest.

Do not hash or disassemble large bitcode in the wrapper. The coordinator performs that work after
the child exits.

Never hardlink a mutable compiler file into the content-addressed store. A later compiler run can
truncate the shared inode.

### Cargo and rustdoc observation

Capture Cargo JSON and accept forwarded non-JSON compiler output. Preserve order inside each stream.
Do not assign an order between streams from independent processes.

Persist sanitized Cargo events by default. Build-script events can contain clear-text environment
values.

Use a separate rustdoc adapter for doctests. Preserve existing builder-wrapper order. Capture
response files and source from standard input.

Compile-only collection uses a recorded no-op test tool for doctests. Build scripts and procedural
macros still run because compilation depends on them.

### Correlation

Cargo, rustc, and the unit graph do not expose one shared stable unit ID. Use evidence in this
order:

1. Exact artifact-path equality.
2. Rustc crate name, source, crate type, target, output directory, and metadata suffix.
3. Cargo package, target, profile, features, mode, and unit-graph platform.
4. Timing as supporting evidence only.

One observation can retain several candidate units. Timing alone never proves an exact
relationship.

Parse every `--extern` name and path. These paths connect consumers to exact producer files. They
also distinguish crate renames or multiple versions.

## Artifact import and storage

Keep exact compiler bitcode as the primary artifact. Use the matching `llvm-dis` during ingestion
to create textual IR for indexing.

The first end-to-end implementation can store raw text. The production store can compress
independent blocks. A logical-offset index supports reads of individual bodies.

Do not change the indexed text because a change invalidates its byte offsets. Compute metrics and
normalized comparisons from a separate representation.

Publish blobs with this protocol:

1. Copy a blob into a temporary content-addressed location, or create a copy-on-write reflink.
2. Compute and validate its digest.
3. Flush the blob.
4. Atomically publish it only if the destination does not exist.
5. Flush the parent directory.
6. Add catalog references in one SQLite transaction.
7. Set the collection status to `complete` in the final statement.

Query only complete collections. A blob without a catalog reference is eligible for later garbage
collection.

Concurrent collectors use separate temporary directories. Only one collector commits to SQLite at
a time. Keep this transaction short. Readers continue to use prior complete records during import.

SQLite uses write-ahead logging and a bounded busy timeout. Each process uses its own database
connection. Queries read only complete collections.

Schema version 4 adds a nullable compiler symbol to each instance. New captures always store this
symbol. Migrated captures keep a null value because their old display text cannot prove the symbol.
The evidence-cache version prevents reuse of captures that used heuristic body matching.

SQLite does not control Cargo output files. An operating-system file lock protects each active
Cargo target cache. Collections that use one cache partition operate one at a time. Collections
with separate cache partitions can operate concurrently.

The catalog records lock ownership and collection status for inspection. The file lock remains the
source of truth because the operating system releases it after a process stops.

The store needs explicit handling for full disks, permissions, cancellation, corrupt blobs, stale
staging, pins, retention changes, and shared-blob removal.

## Query performance

SQLite stores metadata, relationships, precomputed summaries, and 64-bit byte ranges. Large compiler
artifacts remain in content-addressed files.

The catalog indexes definition paths, collection IDs, raw symbols, relationship endpoints, and
common filters. The importer computes function sizes, instruction counts, calls, and opcode counts
before it publishes a collection.

A body query reads one indexed range. It does not parse or load the complete module. A comparison
uses stored summaries before it reads raw body text.

The first CLI opens SQLite directly. A daemon is not part of the initial design. A later version can
add one after measurements show that process startup or database access limits query speed.

## Retention

Use these retention tiers:

- `final` keeps final modules, objects, source, mono evidence, and remarks.
- `pipeline` also keeps pre-pass and LTO transition artifacts.
- `compiler-performance` adds type layouts, phase timing, LLVM traces, or self-profile data.
- `mir-pipeline` adds selected MIR pass dumps for local bodies.

Index an artifact before the retention policy removes it. If body text is unavailable, the retained
summary must state that fact.

## Source snapshots

Source evidence needs three observations:

1. A pre-build source baseline.
2. Compiler dep-info or a debug checksum.
3. A post-invocation validation read.

Use content digests as identity. Paths provide provenance and display values.

Classify each input as workspace, generated, registry, Git, path dependency, sysroot, macro
expansion, unknown generated, or external. Apply an explicit path policy to each class.

Parse dep-info with Makefile escape rules. Preserve native path bytes and store a separate display
string.

Report `validated`, `changed-during-build`, `checksum-mismatch`, `missing`, or `unverifiable`. Never
substitute the current worktree file for a missing historical snapshot.

Label macro and build-script provenance as incomplete. These programs can read inputs that the
compiler does not report.

## Comparisons

Before a cross-build comparison, compare:

- Definition origin and crate target.
- Rust path and generic arguments.
- Target triple and target specification.
- Compiler commit and backend.
- Effective code-generation configuration.
- Capture stage and fidelity.

If information is missing, return all structural candidates. Do not select one candidate by a short
name.

Default comparisons show:

- Function and native byte size.
- Basic-block, instruction, call, and opcode counts.
- Direct callees, indirect calls, and referenced globals.
- Linkage, alias, and body-presence changes.
- Mono estimates and instance counts.
- Inline decisions and surviving inline locations.

After these summaries, show the raw LLVM text. If the compiler stages and configurations are
compatible, compare the text.

## Security boundary

Treat `.optic` as sensitive build output. Use private directory and file permissions.

Sensitive data can appear in:

- Compiler and linker arguments.
- Response files and custom target specifications.
- Build-script environment events.
- Generated or included source files.
- Diagnostics, LLVM constants, and debug information.

Store secret-like environment values as digests by default. Never upload a collection implicitly.

Hash validation detects corruption but does not isolate a hostile build. Use an operating-system
sandbox for untrusted projects.

Apply limits for bytes, files, metadata depth, diagnostics, and parser recursion during ingestion.
