# Persistent storage

This document explains how Cargo Optic stores, reuses, validates, and removes compiler evidence.
Read the [product overview](overview.md) first for the goals and core terms.

## Why the store exists

Compilation is often the most expensive part of a compiler investigation. Artifact discovery and
instance lookup also repeat after each build.

Cargo Optic stores completed captures in the Cargo workspace. Later commands can query the same
evidence without another selected-target compilation.

The store contains compiler observations. It does not replace the Cargo target directory or act as
a general build cache.

## Workspace layout

Each Cargo workspace uses this layout:

```text
.optic/
|-- locks/
|   |-- operation.lock
|   |-- schema.lock
|   |-- writer.lock
|   +-- evidence.lock
+-- store/
    |-- catalog.sqlite
    |-- blobs/
    |-- pending/
    +-- work/
```

The SQLite catalog stores metadata, relationships, summaries, and byte ranges. Content-addressed
blobs store large LLVM modules, source snapshots, and raw remark files.

The lock directory remains outside `.optic/store`. This separation lets `clean` coordinate with
other commands while it removes the store.

Cargo Optic does not create a `.optic.lock` file in the workspace root.

## Immutable captures

A completed capture never changes. A new build or evidence request creates a new capture or reuses
an existing capture after a freshness evaluation.

The product does not have a current capture. Each read command names a capture or instance with an
opaque public ID.

Public IDs do not expose SQLite row identifiers or artifact paths. An instance ID also identifies
its capture.

## Large evidence

LLVM modules can be much larger than available memory. Cargo Optic keeps these modules as immutable
files.

The LLVM index stores 64-bit byte ranges for function bodies. A `show` query reads one function
range instead of parsing the complete module. The reader returns source and LLVM text in chunks of
at most 64 KiB.

Content-addressed blobs let captures share identical evidence. A BLAKE3 digest of the logical,
uncompressed bytes identifies each blob and supports later validation. Schema 10 stores every blob
as one zstd frame without changing logical body ranges.

The schema defines one codec for every blob. It does not add a blob catalog, physical digest,
codec field, or logical byte count.

## Capture reuse

A matching completed capture is only a reuse candidate. Cargo Optic asks Cargo to evaluate the
saved analysis fingerprint before it returns that capture.

The fingerprint represents the selected target with its evidence arguments. These arguments give
the selected target a separate Cargo identity.

Dependencies still use the normal Cargo graph and target directory. Normal builds and Optic builds
can reuse compatible dependency artifacts.

The `--fresh` option requests new evidence after pending-evidence recovery. A successful fresh
capture stores its analysis fingerprint for later normal reuse.

## Publication

New evidence remains outside the visible catalog during compilation and ingestion. Cargo Optic
writes immutable blobs before it starts publication.

Evidence ingestion writes records incrementally to a private staging catalog. One short SQLite
transaction publishes all catalog rows for the completed capture. A reader sees the complete
capture or no capture.

Publication constrains body, declaration, alias, and availability relationships to one capture.
An equal raw symbol in an older capture cannot become evidence for a new instance.

This order prevents a query from reading partial compiler evidence.

## Pending evidence recovery

Compilation can succeed before evidence ingestion fails. Cargo Optic retains validated analysis
artifacts and a bounded marker below `.optic/store/pending`.

A matching request validates the recorded source inputs. Then it asks Cargo to evaluate the saved
analysis fingerprint.

If Cargo reports fresh evidence, Cargo Optic resumes ingestion without another selected-target
compilation.

If tracked inputs changed, Cargo Optic removes the stale pending evidence and compiles again.

Malformed pending data causes an error and remains available for diagnosis. Successful publication
removes its pending evidence.

## Concurrency

SQLite uses write-ahead logging. Read commands can query completed captures while another process
prepares evidence.

Filesystem locks protect operations that also change files:

- The operation lock prevents `clean` from removing a store that another command uses.
- The schema lock protects store creation and schema validation.
- The writer lock serializes capture publication.
- The evidence lock protects removal and garbage collection.

Each active capture uses a private work directory and analysis fingerprint. Content blobs publish
through atomic rename operations.

## Lifecycle commands

The store has explicit lifecycle commands:

| Command | Effect |
| --- | --- |
| `status` | Shows completed captures, pending evidence, blobs, and total bytes. |
| `verify` | Reads referenced blobs and validates their BLAKE3 digests. |
| `remove` | Removes one capture from the catalog. |
| `gc` | Removes blobs that no completed capture references. |
| `clean` | Removes all stored Optic evidence from the workspace. |

The `remove` command does not immediately remove shared blobs. The `gc` command removes a blob only
after no completed capture references it.

The `clean` command removes `.optic/store` only. It preserves `.optic/locks`, durable configuration,
and the Cargo target directory.

## Storage limits and foreign access

The prototype has no labels, pins, automatic eviction, or retention policy. It uses a soft storage
admission policy and explicit lifecycle commands.

The default retained-byte limit is the smaller of 32 GiB and 25 percent of filesystem capacity.
The default free-space reserve is the smaller of 10 GiB and 5 percent of filesystem capacity. A
workspace can set `store.max_bytes` in `.optic/config.toml`. Capture and build-based `show` can
override the limit with `--max-store-bytes`.

Cargo Optic checks the policy before compilation, while it publishes blobs, and at bounded
intervals during ingestion. This policy is not a reservation or hard retained-state bound. Rustc
and temporary files can exceed it between checks. A failure can leave pending evidence or an
unreferenced blob for explicit cleanup.

`status` reports physical retained bytes, referenced and unreferenced blob bytes, pending bytes,
the effective limit, available filesystem space, and the free-space reserve. It does not report
logical decoded bytes.

An explicit `--optic-dir` path opens another workspace's store for read commands. The connection
does not make durable Optic-state changes. SQLite can create or update transient WAL and
shared-memory sidecars while it opens the catalog.

LLVM ingestion removes generated `.ll` files after the store no longer needs them. The MVP does not
add durable per-module ingestion checkpoints.

The remark collector accepts at most 4 GiB per capture, 512 MiB per file, and 5,000,000 YAML
documents.

The current store schema is version 10. The prototype does not detect or migrate older formats.
Running a newer version against older state can return an error or panic.

Read [federated evidence and storage admission](federated-storage.md) for the complete contract.

Read [capture and evidence](capture.md) for the build and freshness inputs. Read
[query and comparison](query.md) for the visible evidence model.
