# Capture and evidence

This document explains how Cargo Optic captures compiler evidence. It also explains the guarantees
that connect the evidence to one real Cargo build.

Read the [product overview](overview.md) first for the goals and core terms.

## Build selection

A capture selects one package and one library, binary, benchmark, or example target. The request
can also select these Cargo inputs:

- A release or named profile.
- A feature list, all features, or no default features.
- A target triple.
- Locked, offline, or frozen Cargo behavior.
- A manifest outside the current workspace of the caller.

Cargo Optic uses the Cargo process that launched it and the default `rustc` from `PATH`. The first
milestone rejects a custom compiler. It disables configured compiler wrappers with a warning.

## Evidence profiles

The evidence profile controls changes that Cargo Optic makes to the selected rustc invocation.

The `faithful` profile is the default. It preserves the code-generation configuration and adds
only the arguments that save evidence.

The `enriched` profile adds v0 symbol names and line-table debug information. This configuration
can provide better source-oriented evidence, but it changes the compiler command.

The `experiment` profile accepts explicit `--rustc-arg` values. It supports a named compiler
experiment without changing the normal project configuration.

Optimization remarks are optional capture evidence. A build-based `show --output remarks` request
enables them automatically.

## Capture sequence

A capture uses the normal Cargo dependency graph and target directory. It does not reproduce the
build in a separate build system.

The capture follows this high-level sequence:

1. Cargo Optic reads metadata with the requested target and feature selection.
2. It rejects a custom compiler and detects configured compiler wrappers.
3. It records the exact rustc and matching LLVM tool identities.
4. It creates a source baseline for workspace packages and selected local path dependencies.
5. It starts `cargo rustc` with the normal dependency graph and target directory.
6. Its outer wrapper passes dependencies, build scripts, and compiler probes through unchanged.
7. The wrapper adds evidence arguments only to the selected target invocation.
8. An exact-version driver records concrete instances during that same rustc process.
9. Rustc continues normal code generation and produces saved LLVM artifacts.
10. Cargo Optic validates the source baseline and ingests the evidence.
11. One short transaction publishes the completed capture.

The driver does not compile the selected target a second time. The first milestone compiles the
small helper program for each capture.

The driver writes compiler-instance records to one private manifest. Cargo Optic reads this
manifest after the compiler process completes.

## Captured evidence

The prototype stores these evidence channels:

- LLVM bitcode before and after optimization.
- Textual LLVM IR from the matching `llvm-dis` tool.
- Concrete instances, definitions, raw symbols, and codegen-unit placements.
- LLVM function definitions, declarations, and aliases.
- Exact source items from validated local source snapshots.
- Requested raw and structured LLVM optimization remarks.
- Cargo and rustc commands, arguments, environment, and compiler provenance.

If ThinLTO evidence exists, the optimized view prefers the `thin-lto-after-pm` stage. Otherwise, it
selects the final supported codegen-unit body.

The source view uses an exact rustc definition span and a canonical local path.

If either value is unavailable or outside an approved package root, the view returns no source.

The source baseline includes the exact Cargo feature selection. This selection includes local path
dependencies that optional features enable.

## Exact instance identity

The rustc driver records each concrete instance and its raw LLVM symbol. It records these facts
during analysis in the selected target compilation.

The LLVM scanner indexes definitions, declarations, and aliases by raw symbol. Equal raw symbols
are the only instance-to-LLVM relationship.

Each relationship also requires the same capture ID. A symbol from an older capture cannot supply
the body, declaration, or alias state for a newer instance.

Display paths support search and output. They never prove an instance-to-body relationship.

This strict rule prevents plausible but incorrect results. It also means that Cargo Optic reports
no standalone body after some LLVM transformations.

## Build fidelity

Cargo Optic uses `cargo rustc` and the normal dependency graph. Cargo can reuse dependency
artifacts from normal builds.

The selected target receives a separate Cargo fingerprint because saved temporary arguments affect
the rustc command. A later normal Cargo command can still reuse compatible dependencies.

Cargo Optic disables existing global and workspace compiler wrappers during capture. It prints a
warning because the compiler output can differ from a normal wrapped build.

The tool records code-generation inputs that affect comparison. These inputs include compiler
environment variables and effective rustc arguments.

Evidence-only rustc arguments do not make two captures incompatible. User and project compiler
arguments remain part of the compatibility result.

## Toolchain selection

The tool uses the stable, beta, or nightly `rustc` from `PATH`. Rustc inspection and driver
compilation start in the selected workspace.

The first milestone supports Unix hosts. It does not support custom compiler commands, Windows
hosts, rustc response files, or temporary filesystems that prevent executable files.

The selected rustc must contain matching `rustc-dev` and `llvm-tools` components. Cargo Optic
reports a clear error for a missing component.

The exact-version driver uses the same `rustc` that Cargo uses for the selected target. The first
milestone does not cache this driver.

The driver returns an error for a non-UTF-8 compiler argument. It does not panic before rustc
starts.

## Unstable access

Cargo Optic sets `RUSTC_BOOTSTRAP` only for these internal operations:

1. Cargo configuration discovery.
2. Exact-version driver compilation.

The selected target, dependencies, build scripts, and compiler probes do not receive unstable
access from Optic. The user does not supply `RUSTC_BOOTSTRAP` for Cargo Optic.

## Freshness

Cargo remains the authority for build freshness. Cargo Optic does not replace Cargo freshness with
a source digest.

A reusable request includes the build selection, compiler identity, target directory, relevant
environment, and evidence version. Cargo Optic asks Cargo to evaluate the saved analysis
fingerprint before reuse.

The source baseline records files from workspace packages and selected local path dependencies. It
also records manifests, lock files, and applicable Cargo configuration.

Cargo tracks declared build-script inputs, included files, and declared environment inputs. Cargo
cannot track an input that a build script does not declare.

The `--fresh` option requests new evidence after pending-evidence recovery.

If no reusable pending evidence exists, the option creates a new analysis fingerprint.

## Output supervision

In text mode, Cargo progress and compiler diagnostics stream to standard error. The final result
uses standard output. In JSON Lines mode, all events use standard output.

Cargo Optic drains standard output and standard error at the same time. A full pipe cannot block
the compiler while Cargo Optic waits for the other pipe.

One Cargo JSON message and the retained failure tail each have a 1 MiB limit. Cargo Optic drains
both output streams before it reports a limit error.

The output supervisor forwards rendered diagnostics and original non-JSON bytes. It retains only
bounded data for artifact freshness and error diagnostics.

If an application consumer stops the event stream, the supervisor cancels the build. On Unix, it
terminates and reaps the Cargo process group, including active rustc descendants.

If a JSON Lines output pipe closes, the CLI stops consuming events. This action uses the same
cancellation path instead of leaving the selected target compilation active.

## Evidence boundary

A capture contains observations from one build. It is not a complete record of all possible build
inputs or compiler transformations.

Cargo cannot find an undeclared build-script input. Source lookup also requires an exact local path
and rustc definition span.

LLVM can remove, clone, merge, or rename a body. Cargo Optic does not infer transformation lineage
without exact evidence.

Read [query and comparison](query.md) for the visible result states. Read
[persistent storage](storage.md) for reuse, recovery, and publication details.
