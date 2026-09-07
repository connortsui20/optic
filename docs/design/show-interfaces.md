# Checkpoint B interfaces

This document records the interface decisions for the completed Checkpoint B.
The decisions refined [narrow show](show.md) without changing its behavior.
Current API names and signatures live on `main`. The assignments below are implementation history.

## Records

Keep one instance list in `InstanceManifest`. Add a required `source: SourceAvailability` field to
`InstanceRecord`. Its constructor accepts source availability after the existing arguments.

| Type | Fields or variants |
| --- | --- |
| `InstanceRef`. | Full `CaptureId` and immutable `u64` ordinal. |
| `ArtifactId`. | Distinct `u64` value, not an instance ordinal. |
| `ArtifactRecord`. | ID, `ArtifactKind::{Source, Llvm}`, and byte length. |
| `ByteRange`. | Start and length, with checked `u64` end. Zero length is valid. |
| `SourceRecord`. | Artifact ID, byte range, display path, and positive starting line. |
| `SourceAvailability`. | `Available(SourceRecord)` or `Unavailable(SourceUnavailable)`. |
| `SourceUnavailable`. | Nonlocal, unloaded, generated/synthetic, unsupported span, or outside package. |
| `LlvmDefinitionRecord`. | Exact raw symbol, byte range, and definition kind. |
| `LlvmDefinitionKind`. | Function, direct alias with exact target, expression alias, or ifunc. |
| `LlvmModuleRecord`. | Artifact ID, compiler module identity, stage, and definitions. |
| `LlvmStage`. | No-LTO optimized output or local-ThinLTO post-pass-manager output. |
| `LlvmCollection`. | `Collected(Vec<LlvmModuleRecord>)` or `NotCaptured(UnsupportedLlvmConfiguration)`. |

Artifact filenames derive only from IDs: `artifact-<16-lowercase-hex-id>`. Do not store a duplicate
filename field or accept arbitrary artifact paths. Modules own their definitions, so neither table
needs another row-ID type. Direct aliases refer only to exact symbols in their own module.

`LlvmProvenance` records backend, optional LLVM version, effective target, optimization, effective
LTO, incremental/linker-plugin status, CGU count, and collection recipe revision. The capture header
already owns Rust compiler identity. Do not duplicate that identity in each module.

The required unsupported LLVM configurations remain those in the show contract. A compiler release
whose artifact-stage mapping has not been verified also cannot claim available optimized LLVM.
Classification must preserve core capture and source when the driver can otherwise collect them.

All record fields remain private. Constructors and deserialization enforce the same invariants.
Manifest validation checks unique artifact IDs and module symbols, artifact kinds, range bounds,
and source/module references. No records perform filesystem or Cargo operations.

The complete constructor is:

```rust
InstanceManifest::new(
    capture_id: CaptureId,
    instances: Vec<InstanceRecord>,
    artifacts: Vec<ArtifactRecord>,
    llvm_provenance: LlvmProvenance,
    llvm: LlvmCollection,
) -> Result<InstanceManifest, RecordError>
```

The durable format, private protocol, and evidence-policy revisions change together. No instance-only
compatibility constructor or implicit source default remains in the final product.

## Compiler and store ownership

`CollectedBuild` owns the private artifact directory until the store copies it. Its consuming
`into_parts(capture_id)` validates the complete manifest and returns these values:

```rust
Result<
    (BuildRecord, CompilerIdentity, CaptureAnalysis, InstanceManifest, tempfile::TempDir),
    CompilerError,
>
```

The capture layer generates the ID and retains that temporary directory through publication.
The store accepts `publish(capture, manifest, artifact_directory)`. It copies only declared generated
filenames, validates file types and lengths, and preserves A's pointer-before-capture commit order.
`read_instances` and candidate validation check every declared artifact, without reparsing LLVM.

In B, `read_candidate` returns `Option<(CaptureRecord, InstanceManifest)>`. Candidate validation
already loads that manifest. Returning it lets capture report stored LLVM unavailability on reuse
without reading a potentially large manifest twice. The capture layer emits that warning only
after accepting reuse, or after publishing a new capture. The store does not print diagnostics.

The store's range reader is:

```rust
Store::copy_evidence(
    &self,
    capture: &CaptureId,
    artifact: ArtifactId,
    range: ByteRange,
    writer: &mut impl Write,
) -> Result<(), StoreError>
```

It uses a fixed buffer and reports premature EOF or writer failure. The public API exposes a
capture-scoped range descriptor instead of separate unscoped arguments or unrestricted paths.

## Evidence and application views

`FindResults::instances()` returns `FoundInstance` values. Each exposes `reference()` and `record()`.
Assign ordinals before sorting or limiting. Carry those original ordinals through sorting, then
construct references for retained results. This preserves identity without cloning the capture ID
for every omitted match. Do not add forwarding getters that duplicate the existing instance API.

The evidence crate owns `EvidenceRange`, containing capture ID, artifact ID, and `ByteRange`.
`SourceEvidence` distinguishes available source with its range/path/line from `SourceUnavailable`.
`LlvmEvidence` distinguishes exact bodies, unsupported collection, no exact standalone definition,
and unsupported exact aliases. Each body retains module/stage and any direct-alias chain.

Alias resolution uses exact same-module names and detects cycles. A cyclic direct alias is malformed
evidence, not proof of elimination or a fallback to another symbol. Return every resolved body in
deterministic module/range order. Never join on display names.

The API re-exports these views and exposes `source(&InstanceRef)`, `llvm(&InstanceRef)`, and
`copy_evidence(&EvidenceRange, &mut impl Write)`. CLI output selection stays in the CLI crate.
Unavailable output writes no stdout. Successful stdout contains only the requested bytes.

The evidence entry points are `source_evidence(store, reference)` and
`llvm_evidence(store, reference)`. Source's available variant contains `evidence`, `display_path`,
and `starting_line`. LLVM uses `Available(Vec<LlvmBody>)`, `NotCaptured`, `NoExactDefinition`, and
`UnsupportedAlias` variants. A body exposes its evidence range, module, stage, final raw symbol,
and aliases in resolution order. When exact bodies exist, return all of them even if another module
contains an unsupported alias. An alias cycle remains an error.

## Parallel implementation

The storage worker owns records and store changes. The compiler worker owns source collection,
configuration classification, artifact-stage proof, disassembly, and private protocol integration.
The test worker owns shared fixtures and process acceptance tests. The integration owner owns
capture/API/CLI composition and documentation.

Two additional bounded workers can own the evidence crate and the pure streaming LLVM indexer.
The indexer owns only `cargo-optic-compiler/src/llvm_index/`, with its module declaration and caller
owned by the compiler worker. It returns validated definition records from a `BufRead` input.
It must obey the fixed-chunk and 1 MiB header bounds in the show contract.

The compiler worker verifies the actual Rust 1.98.1 artifact stage before making LLVM available.
The integration owner reads each handoff and repeats all A journeys after B integration.
