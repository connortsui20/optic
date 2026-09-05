# Federated evidence and storage admission

This document defines the federated-storage slice of the Cargo Optic prototype. The slice keeps
each writable store in one Cargo workspace. It adds explicit read-only access to another `.optic`
directory.

This design also adds a soft storage-admission policy. It keeps SQLite for metadata and uses
compressed, content-addressed files for large compiler artifacts.

The [persistent storage design](storage.md) describes the current store. The
[`cargo-optic` design](cargo-optic.md) describes the existing commands and application API.

## Decision

Cargo Optic does not create a global writable store. A capture remains owned by the workspace that
created it.

The tool supports _foreign evidence_. Foreign evidence is a completed capture in an explicit
`.optic` directory. Cargo Optic opens this evidence in read-only mode.

This model gives two worktrees access to each other's completed evidence. It does not merge their
catalogs or share capture writes.

SQLite remains the catalog. Immutable files remain the large-data layer. The prototype does not
add Vortex, Iceberg, a daemon, or a shared object service.

Large blobs use streaming zstd at its default compression level. Cargo Optic does not define a
compression algorithm.

## Goals

This slice has these goals:

1. Read completed evidence from another worktree without Cargo workspace discovery.
2. Compare instances from two explicit stores.
3. Prevent a foreign reader from making durable Optic-state changes.
4. Report the physical size of retained evidence.
5. Reject new capture work when a storage-policy checkpoint fails.
6. List, inspect, and remove retained pending evidence.
7. Increase the bounded limits for large remark sets.

The slice does not provide automatic store discovery. It also does not provide shared writes,
cross-store deduplication, automatic eviction, labels, or pins.

## Store roles

The application has two store roles.

A _workspace store_ belongs to the discovered Cargo workspace. It supports capture, reads, and
lifecycle commands.

A _foreign store_ comes from an explicit `.optic` path. It supports reads of completed evidence and
pending metadata. It does not support capture or lifecycle mutations.

The typed API makes this distinction explicit:

```rust
Application::discover(start: &Path) -> Result<Application>
Application::open_read_only(optic_dir: &Path) -> Result<Application>

Application::compare_with(
    &self,
    before: &InstanceId,
    other: &Application,
    after: &InstanceId,
    output: CompilerOutput,
) -> Result<CompareView>
```

`discover` keeps the current Cargo discovery and writable-store behavior. `open_read_only` does not
run Cargo metadata.

The prototype keeps one `Application` type. Its internal store role rejects operations that
require write authority.

## Command interface

The `--optic-dir` option selects one foreign `.optic` directory for a read command:

```console
cargo optic --optic-dir ../other-worktree/.optic captures
cargo optic --optic-dir ../other-worktree/.optic show --instance INSTANCE_ID
```

Foreign access supports these commands:

- `list-captures` lists completed captures.
- `find` searches one completed capture.
- `show --instance` shows stored evidence.
- `inspect` shows stored provenance.
- `status` reports retained state.
- `verify` verifies referenced blobs.
- `pending` and `pending inspect` show retained pending evidence.
- `compare` reads one or both sides from explicit stores.

A foreign store rejects `capture`, build-based `show`, `remove`, `gc`, `clean`, and
`pending remove`. These commands require ownership of the workspace store.

Comparison has one optional store path for each side:

```console
cargo optic compare \
  --before INSTANCE_A --before-optic-dir ../old/.optic \
  --after INSTANCE_B --after-optic-dir ../new/.optic
```

A missing side path uses the global `--optic-dir`. If neither option supplies a path, the command
uses the current workspace store.

Two side paths let `compare` run outside a Cargo workspace. The comparison uses typed summaries
from both applications. It does not attach SQLite databases or parse command output.

An ambiguous result prints a complete follow-up command. The command retains the canonical
`--optic-dir` path and the selected output options.

The prototype does not add short store names. Scripts can provide their own path aliases.

## Read-only contract

`open_read_only` requires an existing `.optic` directory, store, catalog, and lock set. It does not
create an Optic directory, lock file, catalog row, blob, pending run, or work directory.

The foreign SQLite connection uses read-only and query-only modes. It rejects an unsupported schema
before it reads evidence. SQLite can create or update transient `catalog.sqlite-wal` and
`catalog.sqlite-shm` sidecars while it opens a WAL database. These files are SQLite coordination
state, not durable Optic evidence.

The reader takes the existing shared operation lock. Evidence reads also take the existing shared
evidence lock. These locks prevent `clean`, `remove`, or `gc` from changing files during a read.

The reader canonicalizes the `.optic` path once. Error messages and generated commands use that
canonical path.

Only completed captures can appear in capture and instance queries. Pending artifacts remain
available through `pending` and `pending inspect`.

## Blob storage

Each blob name is the BLAKE3 digest of its logical, uncompressed bytes. This rule lets equal
logical content share one blob. Schema 10 defines every blob as one zstd frame. It does not add a
blob catalog, physical digest, codec field, or logical byte count.

LLVM body ranges continue to refer to logical text offsets. A streaming zstd reader skips decoded
bytes before the requested range. It emits the selected body in chunks of at most 64 KiB.

This MVP accepts the decode cost for a range read. It does not add a seekable codec.

Normal reads decode the frame and verify the logical digest. The zstd frame checksum detects
encoded corruption. A later codec change requires another store-schema version.

## Storage accounting and budget

`status` reports physical bytes. It reports total retained bytes, referenced and unreferenced blob
bytes, and pending bytes. It does not report logical decoded bytes.

The retained total includes these categories:

- Referenced blobs belong to completed captures.
- Unreferenced blobs await `gc`.
- Pending bytes belong to retained recoverable evidence.
- Work bytes belong to active capture or ingestion work.
- Catalog bytes include SQLite database and journal files.

The first configuration entry is:

```toml
[store]
max_bytes = "32GiB"
```

`capture` and build-based `show` also accept `--max-store-bytes`. The command option overrides the
workspace configuration.

Without configuration, the default limit is the smaller of 32 GiB and 25 percent of the filesystem
capacity. The free-space reserve is the smaller of 10 GiB and 5 percent of the filesystem capacity.

The parser accepts unsigned integers and the `KiB`, `MiB`, `GiB`, and `TiB` suffixes. It rejects
decimals, unknown keys, duplicate keys, and overflow.

The policy is a soft admission check. It is not a reservation, retained-state bound, or filesystem
quota. Cargo Optic checks it before compilation, while it publishes blobs, and at bounded intervals
during ingestion. Rustc and one compressed staging blob can write between checks.

A budget error can leave recoverable pending evidence or an unreferenced blob. It never removes a
completed capture. Explicit-ID reads, `remove`, `gc`, and `clean` remain available when a store
exceeds its limit. Capture and build-based `show` requests check admission before Cargo evaluates a
completed capture for reuse, so capture reuse is also unavailable while the policy fails.

## Pending evidence

Each retained pending request receives an opaque `PendingId`. Cargo Optic replaces the `cap_` prefix
of the reserved capture ID with `pen_`.

The local workspace supports these commands:

```console
cargo optic pending
cargo optic pending inspect PENDING_ID
cargo optic pending remove PENDING_ID
```

The summary contains the reserved capture ID, build request, compiler versions, and retained bytes.

`pending remove` uses the workspace lifecycle locks. It removes the selected pending directory.

Malformed pending metadata remains inspectable by path and error. It does not become a completed
capture.

## Ingestion cleanup

Ingestion processes one LLVM module at a time:

1. It disassembles and indexes the module.
2. It publishes the bitcode and text blobs.
3. It commits the module rows to the private staging catalog.
4. It removes the generated `.ll` file after the store no longer needs it.

The retained compiler artifacts remain the recovery source. A resumed request repeats ingestion
from these artifacts.

The MVP does not add durable module or remark checkpoints. It only removes generated `.ll` files
that `llvm-dis` can recreate.

The collector accepts at most 4 GiB of remarks for one capture. One remark file can contain at most
512 MiB, and one capture can contain at most 5,000,000 YAML documents.

Publication still uses one short transaction. Readers see a complete capture or no capture.

## Versions and errors

Schema 10 defines the compressed blob representation. The pending marker remains version 1 because
its stored representation does not change.

The prototype does not detect or migrate older formats. Local and foreign access can return an error
or panic when it reads older state.

Stored paths, byte counts, compressed frames, and pending metadata are untrusted input. Invalid
data returns an error. Internal invariants remain the only panic conditions.

The JSON Lines version remains 1. New status fields and pending events are additive.

## Deferred work

The following features remain outside this slice:

- A global registry of store paths remains deferred until repeated path entry becomes a measured
  problem.
- Cross-store blob deduplication remains deferred because it needs shared write ownership.
- Automatic eviction, labels, and pins remain deferred because explicit lifecycle commands are
  sufficient for the prototype.
- A generic-definition inventory remains deferred because rustc does not report definitions that
  have no concrete instances.
- Target-owner guidance remains deferred until the capture protocol records enough definition
  ownership data.
- Vortex and Iceberg remain deferred because the current queries do not require an analytical table
  engine or a distributed table manifest.

## Acceptance boundary

The MVP is complete when two worktrees can query and compare completed evidence through explicit
paths. A foreign read must not make a durable Optic-state change. SQLite can manage transient WAL
and shared-memory sidecars.

The MVP is also complete when capture work stops at a failed storage-policy checkpoint. The store
must report its physical retained bytes, blob categories, pending bytes, effective limit, available
filesystem space, and free-space reserve.

The remark collector must enforce the new aggregate, file, and record limits. An interrupted
ingestion must retain enough state for inspection, removal, or resume.
