# Possible future work

This document records product ideas from prototype use. It is not a roadmap or a compatibility
contract. A later implementation can use different commands, storage, and compiler boundaries.

Each idea needs new design work and validation. The evidence came from the Cargo Optic and RowFn
investigation on 2026-08-22. It used Vortex revision
`d6060eed3a6b298d1cf579db840365f1454e922b`.

## Findings with strong prototype evidence

### Select evidence before LLVM disassembly

One Vortex library capture contained 103,041 instances and 129 bitcode modules. The capture saved
3.326 GiB of bitcode.

ThinLTO left only 33 modules queryable for the requested stage. Ingestion still disassembled every
supported module and produced 18.442 GiB of logical LLVM text. One failed run retained approximately
23 GiB of pending state.

Streaming ingestion, compressed blobs, and storage admission limit the retained damage. They do not
remove the unnecessary disassembly and indexing work.

A build-based `show` can use its query before LLVM disassembly. The identity manifest can identify
candidate instances, placements, codegen units, and compiler stages.

This design needs an explicit evidence model. A query-scoped result must not look like a complete
capture. Ambiguous queries can also require evidence from more than one module.

The separate `capture` command can remain the explicit full-capture workflow. New measurements must
compare full and query-scoped ingestion with identical build dimensions.

### Add native-code evidence

The RowFn investigation used LLVM IR to find vector predicates and packed masks. LLVM IR did not
prove final mask-register use or linked code size.

That work required separate compiler builds and manual symbol correlation. Assembly linked to an
exact compiler instance removes this gap.

Assembly is the first useful native-code output. Object evidence and linked-product symbols are
separate extensions with different identity and lifecycle requirements.

Native-code output must retain the exact build provenance and instance relationship. Static output
must not become a runtime-performance claim.

### Explain target ownership

A benchmark-target capture omitted useful dependency-owned instances in the Vortex investigation.
The useful instances belonged to the `vortex-array` library compilation.

A no-match result can explain which target Cargo Optic captured. Recorded ownership evidence can
also support an exact suggestion for another target.

The capture protocol needs enough definition and target ownership data before the CLI can give exact
guidance. A generated command is better than a package-name guess.

Automatic dependency capture is a separate feature. It increases compilation, ingestion, and
storage work, so it must not be the default answer to missing ownership evidence.

## Conditional ideas

These ideas have plausible uses, but the prototype evidence does not establish their priority.

### Portable evidence bundles

Foreign stores support read-only worktree access and cross-store comparison. They do not define a
portable archive or transfer format.

An export can package selected captures, referenced blobs, and provenance. An import can validate
the schema, IDs, and content digests before it exposes the evidence.

Copying a complete `.optic` directory is the current workaround. A bundle format needs a concrete
cross-machine, archival, or sharing workflow before it adds another storage contract.

### Wider cross-worktree reuse

Explicit foreign stores solve inspection and comparison across worktrees. Captured source also
removes the need for source-root remapping during stored-source display.

Shared capture writes, cross-store deduplication, and live-source overlays remain different
features. They require write ownership, locking, and source-identity rules.

These features need a demonstrated workflow before Cargo Optic expands beyond workspace-owned
writable stores.

### A dedicated driver-cache path

The exact-version driver cache lives below `$CARGO_HOME/optic/drivers`.
`CARGO_HOME` can relocate it, but that variable also relocates the rest of Cargo state.

A dedicated cache path can help read-only home directories and shared CI caches. This option needs
a repeated product problem, not one restricted development environment.

## Capabilities to preserve or replace deliberately

The prototype already implements several ideas from the same investigation. A replacement design
does not need the same implementation, but it needs an explicit decision for each capability:

- Read completed evidence from an explicit foreign store and compare instances from two stores.
- Use symbol-first indexes or set-based joins for large availability calculations.
- Keep normal cache admission cheap and reserve complete digest scans for explicit verification.
- Selected evidence reads retain full content-digest verification.
- Report storage use and reject new capture work at bounded admission checkpoints.
- Show capture and ingestion progress through text and versioned events.
- List, inspect, and remove one retained pending run.

These items are not future backlog for the current prototype. They are requirements discovered by
prototype use and can inform the next product design.

## Existing deferred directions

MIR, inline attribution, more target modes, and a TUI remain possible product directions. The
prototype does not provide enough evidence to rank them with the findings above.

The [federated storage design](federated-storage.md) records related storage deferrals. The
[product overview](overview.md) records the current feature and product boundaries.
