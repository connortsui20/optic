# Query and comparison

This document explains how Cargo Optic finds instances and shows captured evidence. It also
explains the meaning of common result states.

Read the [product overview](overview.md) first for the goals and core terms.

## Output selection

The `show` command returns one compiler output for one concrete instance. Optimized LLVM IR is the
default output.

The command supports these output values:

- `llvm` shows saved LLVM IR after optimization.
- `llvm-pre-opt` shows LLVM IR before the LLVM optimization pipeline.
- `remarks` shows saved LLVM optimization remarks.

The `--source` option adds the captured Rust item to any supported output.

The application streams source and LLVM text in chunks of at most 64 KiB. Terminal syntax
highlighting keeps its parser state across chunks, including partial lines and block comments.

## Optimized LLVM selection

Rustc can save several LLVM artifacts for one instance. Cargo Optic records the exact stage and
codegen-unit provenance for each artifact.

If ThinLTO evidence exists, optimized output prefers the `thin-lto-after-pm` stage. Otherwise, the
output uses the optimized codegen-unit artifact.

The pre-optimization view uses the saved artifact from before the LLVM optimization pipeline.

## Source results

The source result comes from the immutable source snapshot in the capture. It does not read the
current workspace file.

Source lookup requires an exact rustc definition span and canonical local path.

If either identity is unavailable, source lookup returns no source.

The source reader rejects closure spans that do not identify an exact source item. It also rejects
paths outside the approved package roots.

## Optimization remarks

A build-based `show --output remarks` request captures remarks automatically. A separate capture
uses `capture --remarks`.

The capture stores raw remark files and typed records. It distinguishes these capture states:

- Remarks were not captured.
- Remarks were captured, but the capture contains no records.
- Remarks were captured, and the capture contains records.

An instance query can still return no remarks from a capture that contains other remark records.

Remark queries support a kind filter, an exact LLVM pass name, and a result limit. The enriched
evidence profile provides available LLVM-emitted source locations.

## Lookup

Lookup first tests exact definition paths, display names, and compiler symbols.

If no exact match exists, lookup uses a case-sensitive literal substring search. This search
requires at least three Unicode characters.

The `find` command can restrict results by crate, definition, and LLVM-output availability. Its
default limit is 50, and its maximum limit is 500.

The substring index treats the query as literal text. Query text cannot insert FTS operators.

JSON Lines results include these lookup facts:

- The match kind.
- The full compiler symbol.
- A short fingerprint of the exact compiler symbol.
- The result truncation state.

## Ambiguous queries

A definition query can match several concrete compiler instances. Cargo Optic does not select one
instance without evidence from the user.

Plain output prints a complete `show --instance` command for each candidate. It preserves the
requested source and output options.

Public IDs have random UUID suffixes. Plain output shows a unique prefix. JSON Lines results return
the full ID.

An instance ID identifies its capture. The `show --instance` command does not need a separate
capture ID.

Body, declaration, and alias availability comes only from modules in that capture. Repeated raw
symbols in another capture cannot change the result.

## Comparison

The `compare` command accepts two exact instance IDs. The user selects both instances because the
prototype does not create a logical identity across captures.

The compatibility result compares these recorded dimensions:

- The rustc release, commit, and host.
- The LLVM version and compiler target.
- The evidence profile and Cargo build request.
- The effective compiler environment.
- The effective rustc arguments after Optic removes evidence-only arguments.

The structural result compares these LLVM properties:

- Body bytes.
- Instruction-like lines.
- Vector lines.
- Safety-check symbols.
- Typed call categories.

Call categories separate runtime calls, indirect calls, inline assembly, memory intrinsics,
assumptions, lifetime intrinsics, metadata intrinsics, and other intrinsics.

The comparison can use optimized or pre-optimization LLVM. Structural counts do not predict
execution time or replace a benchmark.

## Cross-store queries

The `--optic-dir` option lets a read command select one explicit `.optic` directory. This path
opens completed evidence without Cargo workspace discovery or durable Optic-state mutation.

`compare` accepts a separate store path for each instance. The application reads typed summaries
from both stores and uses the existing comparison logic.

An ambiguous result retains the selected store path in each complete follow-up command. The
prototype does not add persistent store aliases or automatic store discovery.

Read [federated evidence and storage admission](federated-storage.md) for the command and locking
contracts.

## Common result meanings

### No standalone body

This result means that the selected stage has no exact body for the recorded raw symbol. It does
not mean that the source function never contributed code.

LLVM can inline or remove the standalone body. A body can also exist at a different captured
stage.

### Remarks not captured

This result means that the build did not request remark evidence. A captured result with no records
has a different state.

### No matching remarks

This result means that the selected instance or filters have no matching records. The capture can
still contain remarks for other instances.

### Compatible comparison

This result means that the recorded compatibility dimensions match. It does not prove equal
runtime behavior or equal external tool contents.

### Reused capture

This result means that Cargo accepted the known build inputs for the saved analysis fingerprint.
The evaluation does not include undeclared build-script inputs.

### Captured source

This result is the immutable source snapshot for the capture. It is not a read of the current
workspace file.

## Interpretation boundary

Cargo Optic reports compiler evidence. It does not turn structural LLVM facts into performance
claims.

A vector line can show that LLVM used a vector type. It does not prove that the complete workload
is faster.

A missing safety-check symbol can support a focused code-generation finding. A relevant benchmark
is still necessary for a runtime-performance claim.

Read [capture and evidence](capture.md) for the identity and freshness guarantees. Read
[persistent storage](storage.md) for the capture lifecycle.
