# What the Optic research established

## Status

This document summarizes the compiler research that controls the current architecture. The
[`design plan`](../design/PLAN.md) defines current product behavior.

The detailed reference documents preserve original experiments and planned extensions. They do not
define the current prototype contract.

## Conclusion

Cargo Optic can record exact compiler evidence from a real Cargo target. No observed compiler
behavior blocks the current LLVM workflow.

The implemented prototype can:

- Use the stable, beta, or nightly rustc that the workspace selects.
- Preserve the normal Cargo dependency graph and target directory.
- Record exact compiler instances during the selected target compilation.
- Capture optimized and pre-optimization LLVM modules.
- Capture structured LLVM optimization remarks.
- Find concrete generic instances through definitions, symbols, and placements.
- Show exact source items and standalone LLVM bodies.
- Compare structural LLVM summaries for two exact instances.
- Reuse completed evidence after Cargo validates freshness.
- Resume validated evidence after a post-compilation ingestion failure.

The prototype does not implement the complete research model. MIR, assembly, object symbols,
inline occurrence navigation, cross-build logical identity, and a TUI remain outside its contract.

## Product and libraries

`cargo-optic` is the only user product. It provides the CLI and the typed application API.

`cargo-ir` is an unpublished internal library. It owns Cargo execution, rustc integration, LLVM
artifact discovery, and bounded parsers.

The `.optic` directory is a shared workspace store, not a session. Stateless commands use explicit
capture and instance IDs.

The current prototype has no client context, labels, pins, retention budget, automatic eviction,
or daemon.

## Compiler background

Only a few compiler concepts control the implemented design.

```text
Cargo target
    -> selected rustc invocation
    -> concrete compiler instances
    -> LLVM codegen units
    -> LLVM optimization and optional LTO
    -> saved compiler artifacts
```

Cargo coordinates the build and decides freshness. It can reuse a prior artifact without starting
rustc.

Monomorphization creates concrete instances of generic functions. One definition can produce
separate instances for different type or constant arguments.

A codegen unit is one compiler partition. Rustc can place one instance in several codegen units.

LLVM IR is a low-level program representation. Bitcode is its binary encoding. ThinLTO can import,
rename, internalize, or remove definitions across partitions.

Inlining moves instructions into a caller. The original standalone body can remain or disappear.

A raw symbol is the compiler-owned low-level function name. An alias provides another symbol for an
existing body.

## Findings that control the architecture

### Faithful capture preserves the requested build

Saved compiler temporaries can retain LLVM pipeline artifacts without a separate source-only build.
The collector uses a private analysis directory for the selected target.

The faithful profile preserves optimization, LTO, codegen-unit count, debug configuration, target
CPU, target features, and panic behavior.

The enriched profile changes symbol mangling and debug information for stronger evidence. The
experiment profile accepts explicit code-generation changes.

### Workspace compiler selection is sufficient

The exact-version driver can run with the workspace-selected stable, beta, or nightly rustc. The
compiler needs matching `rustc-dev` and `llvm-tools` components.

Cargo Optic uses scoped internal unstable access. It does not require a nightly toolchain prefix or
a user-provided `RUSTC_BOOTSTRAP` value.

The unstable-access scopes cover Cargo configuration discovery, driver compilation, and the selected
target. Dependencies, build scripts, and compiler probes do not receive Optic's value.

### Compiler identity needs an exact driver

Display names do not reliably connect rustc instances to LLVM bodies. Vortex PR #9398 demonstrated
this failure with a public path and a canonical module path.

The exact-version driver records the definition path, concrete display name, raw symbol, and
codegen-unit placements. It runs during the selected target compilation.

The LLVM scanner joins a body only through an equal raw symbol. A display name can rank search
results, but it cannot create an evidence relationship.

### Each LLVM stage is different

The phrase `optimized LLVM IR` does not identify one universal artifact. Saved stage names depend on
the LTO configuration and rustc version.

A function can change its signature, linkage, symbol, or body between stages. LLVM can also import,
merge, or remove it.

Cargo Optic stores the exact compiler stage for each module. Optimized queries prefer an available
final ThinLTO pipeline stage.

### Large artifacts require bounded readers

The original scale experiment produced a 1.7 GB fat-LTO text module. An owned parse of the complete
module is not acceptable.

The LLVM index stores 64-bit byte ranges. A body query reads one range and does not load the
complete module.

The compiler identity manifest also uses bounded records. The reader streams a manifest up to the
aggregate limit from one open file.

### Optimization remarks need explicit states

LLVM can write no remark records for a successful capture. An empty result does not mean that
capture failed.

The product distinguishes remarks that were not captured, captured remarks with no records, and
captured remarks with records.

Remark records use exact Function symbols. An exact symbol can connect one record to several
compiler instances, and unlinked records remain stored.

### Cargo freshness remains authoritative

Wrapper arguments do not replace Cargo's fingerprint system. Cargo Optic gives the selected target
a separate analysis fingerprint.

Before reuse, Cargo validates that fingerprint through the normal target directory. This process
includes Cargo-tracked files and declared build-script inputs.

Cargo cannot track an undeclared build-script input. The user must request fresh evidence after
such an input changes.

### Compiler wrappers affect the build

`RUSTC_WORKSPACE_WRAPPER` changes Cargo artifact hashes. Optic therefore preserves existing global
and workspace wrappers in their original order.

The outer wrapper passes compiler probes, dependencies, and build scripts through without evidence
arguments. Only the selected target receives the driver and analysis arguments.

### Source provenance is incomplete

Cargo metadata and source scanning identify workspace packages and local path dependencies. The
driver provides an exact source span for supported definitions.

Build scripts and procedural macros can read undeclared inputs. Cargo Optic cannot promise a
complete source or input history without an external sandbox.

Compiler arguments, source, diagnostics, and LLVM constants can contain sensitive data. The local
store remains private build output.

## Current evidence model

The current catalog keeps these facts separate:

- Capture request and compiler provenance.
- Rust definition.
- Concrete compiler instance.
- Codegen-unit placement.
- LLVM module stage.
- Function body, declaration, and alias.
- Source blob and exact item span.
- Raw remark file and typed remark record.
- Exact remark-to-instance relationship.

One compiler instance can have several placements and stage records. A missing exact body remains a
valid result.

The model does not claim one logical identity across captures. Cross-capture comparison uses two
explicit instance IDs and reports incompatible build dimensions.

## Current storage model

The workspace uses `.optic/store` for the catalog, blobs, pending evidence, and private work. It
uses `.optic/locks` for durable coordination files.

Completed captures are immutable. Large LLVM modules remain content-addressed blobs, and catalog
rows store their byte ranges.

`clean` removes only `.optic/store`. It preserves `.optic/locks` and future durable workspace
configuration below `.optic`.

Post-compilation ingestion failures can retain pending evidence. A matching request validates Cargo
freshness before it resumes ingestion.

## Current product boundary

The supported target selection covers one library, binary, benchmark, or example. The automated
acceptance workflow runs on Apple silicon macOS.

The product supports optimized LLVM IR, pre-optimization LLVM IR, and optimization remarks. It uses
plain text and versioned JSON result formats.

The product does not support:

- MIR.
- Assembly.
- Object or linked-product evidence.
- Inline occurrence navigation.
- Rustdoc or doctest capture.
- A TUI, daemon, or persistent client context.
- Automatic cross-build instance matching.
- Complete macro or build-script provenance.
- Non-LLVM code-generation backends.

## Later research areas

Later work can evaluate MIR, assembly, object symbols, inline attribution, more Cargo target modes,
and more platforms. Each addition needs a concrete investigation and a contract test.

Direct LLVM integration is useful only for a clone, merge, or rename that prevents an exact query.
A complete reachable-function graph needs more rustc integration.

The historical reference documents describe broader ideas. Those ideas are not commitments for the
current product.
