# Cargo Optic architecture

The CLI calls the `optic` application API. The API composes capture, search, and stored evidence
reads without moving compiler or filesystem policy into the command handlers.

## Subsystems

| Package | Responsibility |
| --- | --- |
| `cargo-optic-records` | Validated durable identities, provenance, and evidence metadata. |
| `cargo-optic-compiler` | Cargo request preparation, exact-version driver, freshness, and collection. |
| `cargo-optic-store` | Local persistence, bounded reads, cache pointers, and atomic publication. |
| `cargo-optic-capture` | Probe/collect/publish orchestration and typed capture outcomes. |
| `cargo-optic-evidence` | Capture-scoped search, availability, and evidence range selection. |
| `cargo-optic-api` | The supported `optic` application boundary. |
| `cargo-optic` | Cargo subcommand arguments, output, and final diagnostics. |

Records do not invoke processes or read files. The store does not invoke Cargo. The compiler does
not publish captures. Applications use the API without coordinating these boundaries themselves.

## Capture lifecycle

Request preparation resolves the explicit target and compiler configuration from the invocation
directory. The driver is compiled against that exact compiler and reused at a stable cache path.
All embedded source, protocol, compiler, and driver-build inputs contribute to its cache key.

The request key selects a possible completed capture. It does not reproduce Cargo's fingerprint
algorithm. Cargo remains responsible for tracked source, dependencies, build scripts, and
configuration changes.

A candidate probe either proves the selected target fresh or stops before selected-target analysis.
Each actual compilation receives a new analysis token. A token associated with published evidence
is never compiled again. This prevents a failed attempt or a configuration change from assigning
different evidence to a previous Cargo build identity.

The capture layer preserves the complete old record on reuse. For new evidence, the store prepares
and validates a private staging directory. It installs the request pointer immediately before
renaming that directory into the completed namespace. No required fallible cache update follows
the commit rename.

A missing pointer or a pointer to an absent capture is a cache miss. Present malformed data is an
error. The store does not search old headers to reconstruct eligibility after a failed publication.

## Driver boundary

Cargo invokes the standalone driver as its compiler wrapper. Discovery calls and nonselected targets
pass through. A selected-target probe stops before analysis. A selected collection runs rustc's
callbacks and completes its private manifest only after successful compilation.

The runtime driver depends on unstable rustc internals. Its protocol is private to one tool release,
separate from the durable store format. Unsupported revisions fail explicitly.

Compiler output stays owned by its collection result until the store has copied the declared
artifacts. Returning paths into a dropped temporary directory would violate publication's contract.

## Evidence identity

A capture ID identifies immutable evidence, not the current source tree. An instance reference
contains that capture ID and the instance's stored ordinal. Search sorting does not change it.

Source spans select bytes from compiler-loaded normalized source snapshots. Generic instances can
share one definition range. Unsupported provenance remains unavailable instead of selecting nearby
source.

LLVM relationships use exact raw symbols within the captured optimized modules. Display names and
demangling serve search, not identity. Multiple exact bodies remain distinct module/range results.
An absent standalone symbol does not establish why LLVM lacks that body.

Known unsupported collection configurations have typed unavailability. Missing artifacts that a
supported configuration requires are errors. This distinction preserves ordinary capture/search
without disguising failed collection.

## Durable trust boundary

Constructors and deserialization establish record invariants. The store validates record agreement,
artifact paths, file kinds, byte lengths, and range bounds. It rejects oversized JSON before
deserialization and enforces the same encoded-size limits before publication.

Artifact copying and evidence reads use fixed-size buffers and checked 64-bit ranges. Warm reuse
does not reparse or hash entire LLVM modules. The store does not promise detection of same-length
malicious edits to artifact contents.

Capture mutation and shared driver provisioning require serialized callers. The MVP provides atomic
publication visibility, not a journal, crash recovery, or filesystem durability protocol.

## Testing

The unpublished test-support crate supplies isolated Cargo workspaces, command environments, and
diagnostics. Product-specific assertions remain with their owning subsystem or integration test.
API scenarios that depend on environment variables run in fresh children, not by mutating the
multithreaded test runner.

Cache tests observe selected compilation and driver builds separately. They also verify failed
publication and configuration transitions. Installed CLI and external-library journeys establish
that the package works without repository-relative files or a preexisting runtime driver.

The detailed acceptance matrix and implementation status remain on `planning`. A unit-test result
does not replace real Cargo behavior or installed-product verification.
