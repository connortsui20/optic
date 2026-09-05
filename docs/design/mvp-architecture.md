# MVP architecture

Cargo Optic records evidence from one selected Cargo target. It reuses a completed capture when
Cargo verifies freshness, then finds concrete instances and reads stored source or optimized LLVM.

This document defines the shared boundaries. [Capture reuse](capture-reuse.md) and
[narrow show](show.md) own their detailed algorithms. The [MVP plan](mvp-plan.md) orders the work.

## Ownership

Keep the seven existing product crates. The unpublished test-support crate is development-only.

| Package | Rust crate | Owns |
| --- | --- | --- |
| `cargo-optic-records` | `optic_records` | Durable records, identifiers, and value validation. |
| `cargo-optic-compiler` | `optic_compiler` | Request resolution, Cargo, driver provisioning, and compiler collection. |
| `cargo-optic-store` | `optic_store` | Local persistence, publication, cache pointers, and checked artifact access. |
| `cargo-optic-capture` | `optic_capture` | Probe/collect/publish orchestration and capture outcome. |
| `cargo-optic-evidence` | `optic_evidence` | Search, evidence availability, and reader composition. |
| `cargo-optic-api` | `optic` | Supported application API and subsystem composition. |
| `cargo-optic` | Binary only. | Argument parsing, plain-text output, and diagnostics. |

The dependency direction stays explicit:

```text
CLI -> API -> capture  -> compiler -> records
           |          -> store    -> records
           -> evidence -> store
```

Records contain no filesystem or process work. The store does not invoke Cargo. The compiler does
not publish captures. The CLI does not decide freshness or evidence availability.

Do not introduce the previously proposed identity, attribution, comparison, lifecycle, or operation
crates in this MVP. Those belong to [future work](future-architecture.md).

## Public workflow

Keep `Optic::open`, request selection, listing, and literal search. Add these contracts together
with their consumers:

- `CapturePolicy::{Reuse, Fresh}` selects normal capture or forced analysis.
- `Optic::capture(request, policy)` returns `CaptureOutcome`.
- `CaptureOutcome::{Captured(CaptureRecord), Reused(CaptureRecord)}` reports the actual result.
- `find` returns `FoundInstance` values containing a capture-scoped reference and instance record.
- `source` and `llvm` accept an `InstanceRef` and return typed availability and artifact ranges.
- `copy_evidence` copies one validated range to an `impl Write` without loading the whole artifact.

The capture crate owns transient capture policy and outcome. The records crate owns durable
identifiers and evidence metadata. The evidence crate owns query result views. The API re-exports
the public vocabulary instead of defining parallel wrappers.

API names can receive local refinements before worker assignments. Changes to these semantics need a
planning update before dependent implementation.

`Optic::open` can still run Cargo metadata to locate the workspace. Listing, finding, and showing
stored evidence must not build, provision the driver, probe freshness, or recapture. Do not create a
second store-opening API solely to eliminate this existing metadata call.

## Compiler boundary

Resolve one explicit package and target from the original invocation directory. Preserve the
existing package, target-kind, profile, and feature options. Preserve Cargo configuration lookup and
ordinary dependency reuse. Unsupported compiler or wrapper configuration has one explicit policy,
not a fallback chain.

Resolve the default compiler to its actual executable and sysroot. Build the embedded driver against
that exact compiler. Record the compiler release, commit, host, and sysroot. The initial supported
and CI-tested compiler is Rust 1.98.1. Do not promise compatibility with arbitrary rustc versions
merely because the driver is compiled at runtime.

The standalone driver entry point must explain these routes: Cargo discovery calls, nonselected
compiler forwarding, selected-target freshness probing, and selected-target evidence collection.
Compiler callbacks document the phase at which evidence becomes available.

Keep the private driver protocol distinct from durable JSON. Use the existing small protocol unless
the additional artifact metadata justifies replacing it. Do not add a serialization dependency
merely to move an unchanged fixed header. Document every marker, revision, field order, and
writer/reader agreement. Test the real process exchange rather than maintaining a second test-only
implementation.

The collected result owns its temporary artifacts until the store has copied them. Do not return
paths into a temporary directory that has already been dropped.

## Publication and storage

Keep the local `.optic/store` and its staging/completed-capture separation. Add mutable cache
pointers beside immutable captures, as specified in [capture reuse](capture-reuse.md).

Before the first Cargo build/probe, initialize `.optic/.gitignore` with `*` and a trailing newline.
This prevents Optic's own output from invalidating default build-script file tracking in Git-backed
packages. An existing identical file needs no write. Preserve different existing contents and return
an actionable initialization error. Do not edit the user's root ignore file or package manifest.

Without Git, Cargo's default package scan already excludes dot-prefixed paths. Explicit package
inclusion or build-script tracking of `.optic` can still make capture invalidate itself.
Document that limitation instead of overriding Cargo's tracking or parsing arbitrary ignore rules.
See [Cargo's package-file tracking](https://doc.rust-lang.org/cargo/reference/manifest.html#the-exclude-and-include-fields).

A completed capture contains its header, instance/evidence manifest, and every declared artifact.
Explicit evidence unavailability is valid. An artifact claimed as available but absent is invalid.

Prepare and validate everything before the final directory rename. That rename is the only capture
commit point. No required fallible cache update follows it. Errors before commit publish no new
capture. Old completed captures remain readable.

This is atomic visibility, not crash durability. Do not add fsync choreography, journal recovery,
cleanup scanning, or rollback after commit. Interrupted staging and superseded driver entries can
remain until the user removes them. Document this disk-growth limitation.

The caller must serialize captures within a workspace and driver-cache provisioning within one Cargo
home, including separate workspaces sharing that home. Concurrent writers and external store
mutation during an operation are unsupported. Do not add locks or claim concurrent mutation safety.

## Durable input boundary

Use the existing serde/serde_json validation model. Establish value invariants through constructors
and deserialization once. Filesystem validation belongs to the store.

Adopt explicit initial product limits:

| Durable input | Maximum encoded length |
| --- | --- |
| Capture header or cache pointer. | 1 MiB per file. |
| Instance/evidence manifest. | 128 MiB per file. |

These are MVP resource budgets, not compiler limits or a bound on total process memory. Name and
document the constants at their owner. Change them only with a fixture or real workload that
demonstrates the need.

Check file length before allocating. Also bound the actual read to the limit plus one byte, then
reject excessive input before JSON deserialization. The second bound covers growth during the read.
Apply the same encoded-size limits when writing, before publication. Do not publish data that the
reader will reject.

Validate capture IDs against their directory/header/manifest, artifact IDs against the artifact
table, and finite byte ranges against actual file lengths. Use checked `u64` arithmetic for offsets.
Artifacts use generated file names, not arbitrary paths from source or compiler output. Reject
symlinks and nonregular artifact files at the storage boundary. Do not add race-resistant
platform-specific traversal for unsupported concurrent mutation.

Copy artifacts and requested ranges through a fixed-size buffer. LLVM files need not fit in memory.
Warm reuse validates metadata and declared file lengths without hashing or reparsing entire LLVM
files. Equal-length malicious artifact edits are not detected by a content-integrity guarantee.

Malformed, unsupported-version, oversized, or internally inconsistent present data returns a store
error. Do not reinterpret corruption as evidence unavailability. Only the explicit absent-pointer
and absent-capture cases in the cache contract are misses.

## Search and errors

Preserve exact matches before case-sensitive substring matches. Preserve deterministic ordering, the
pre-limit match count, empty results, and the existing result-limit behavior.

Display names are search text, not identity. Raw symbols establish only the exact LLVM relationships
described in [narrow show](show.md). Capture IDs retain their documented reverse-hex representation.

Use the existing crate error conventions. Add context at subsystem boundaries without flattening
typed availability into strings. Return errors for invalid input and failed operations. Reserve
panics for violated internal invariants.

Unsupported prototype formats need no migrations. Update version constants and all consumers in the
integrated change. Explain what each revision identifies, rather than renumbering it for appearance.

## Contributor documentation

Main must contain a concise architecture guide matching the implementation, a limitations section,
and contributor instructions with a checked-in Rust style reference. Explain entry points and
non-obvious correctness arguments. Do not duplicate this entire roadmap into source comments.

Review the complete affected concepts under `$rust-style`, including the standalone driver. The
existing crate boundaries are the user's explicit exception to the skill's future-caller rule. They
do not justify speculative abstractions inside those crates.
