# Capture and compiler reference

This document contains compiler details that can affect capture fidelity, identity, or product
claims. [`../core.md`](../core.md) contains the required conclusions.

This document preserves the original nightly research. Its scopes, profiles, and planned evidence
channels do not describe the complete current product contract.

## Test environment

The main experiments used:

- Rustc `1.100.0-nightly (34baba539 2026-08-16)`.
- LLVM `23.1.0` from the same Rust toolchain.
- Cargo `1.100.0-nightly (8a0d8afb 2026-08-11)`.
- An `x86_64-unknown-linux-gnu` host.
- A `wasm32-unknown-unknown` cross target.

The fixture is in [`../fixtures/codegen/`](../fixtures/codegen/). The local results are evidence for
this compiler and fixture, not a stable rustc contract.

The sysroot is the compiler-owned directory that contains the Rust standard library and related
tools.

## Faithful capture configuration

The original flag-only experiment added these flags to invocations that requested code generation:

```text
-Csave-temps=yes
-Ztemps-dir=<unique invocation directory>
--emit=mir
-Zprint-mono-items=yes
-Zdump-mono-stats=<unique invocation directory>
-Zdump-mono-stats-format=json
-Cremark=inline
-Zremark-dir=<unique invocation directory>
```

The complete flag set produced the same executable bytes as a control build. Seven final ThinLTO
bitcode files also matched. A separate MIR-only experiment produced the same result.

This result does not permit the adapter to change debug information, mangling, optimization, LTO,
CGU count, panic behavior, target CPU, or target features.

`-Zprint-type-sizes`, compiler phase timing, LLVM time traces, and rustc self-profile remain
optional. They can create large outputs that many function queries do not need.

Metadata-only `cargo check` invocations require another path. The adapter must not add MIR, remarks,
mono statistics, or saved LLVM stages to those invocations.

The adapter does not use `-Zprint-mono-items=yes` for identity. An exact-version rustc driver
records the raw symbol for each compiler instance during the selected target compilation.

## Capture profiles and scope

A fidelity profile states whether capture changes the requested build. Capture scope selects the
compiler units that receive evidence flags. These two choices are independent.

The supported fidelity profiles are:

- `faithful` preserves the requested code-generation configuration.
- `enriched` adds line-table debug information or v0 mangling for stronger identity evidence.
- `experiment` changes compilation behavior for a named investigation.
- `artifact-only` indexes existing objects or linked products without compiler evidence.

The supported scopes are:

- `selected` instruments only the selected package targets.
- `workspace` instruments all workspace compiler units in the selected Cargo graph.
- `closure` also instruments dependency, host, target, registry, Git, and path units.

The outer wrapper observes every rustc invocation. It adds evidence flags only after it identifies
an invocation as in scope.

## Saved LLVM stages

The default local-ThinLTO build retained these files for each source CGU:

| Suffix | Observed meaning |
| --- | --- |
| `.no-opt.bc` | LLVM input before the normal optimization pipeline. |
| `.bc` | Compiler-saved optimized bitcode. Its position depends on LTO mode. |
| `.thin-lto-input.bc` | Input selected for local ThinLTO. |
| `.thin-lto-after-resolve.bc` | Module after symbol resolution. |
| `.thin-lto-after-internalize.bc` | Module after LLVM makes selected symbols private. |
| `.thin-lto-after-import.bc` | Module after ThinLTO imports definitions. |
| `.thin-lto-after-rename.bc` | Module after ThinLTO renames symbols. |
| `.thin-lto-after-pm.bc` | Module after final ThinLTO optimization. |
| `.o` | Object produced from the final module. |

These suffixes are compiler internals. The adapter must retain unknown suffixes and use versioned
rules for known suffixes.

The plain `.bc` suffix does not identify one universal stage. Its meaning changes with the LTO
configuration.

Cross-crate ThinLTO produced 27 input partitions and 196 bitcode files for one small final binary.
The partitions included local, dependency, sysroot, and standard-library code.

The fat-LTO fixture produced these important stages:

| Suffix | Definitions | Declarations | Observed meaning |
| --- | ---: | ---: | --- |
| `.no-opt.bc` | 65 | 10 | Local module before LLVM optimization. |
| `.lto.input.bc` | 2,550 | 262 | Merged local, dependency, and sysroot input. |
| `.lto.after-restriction.bc` | 2,550 | 262 | Merged module after symbol restriction. |
| `.bc` | 409 | 88 | Final optimized fat-LTO module. |

The final textual module was approximately 14.0 MB. A previous scale experiment produced a 1.7 GB
module. Production indexing needs bounded memory and 64-bit offsets.

Direct `--emit=llvm-ir` remains a separate capture mode. It can change the LTO path and partition
set. The stored artifact must record this capture method.

## Matched LLVM tools

LLVM bitcode is not reliably readable by another LLVM version. System LLVM 22 rejected bitcode from
the tested rustc LLVM 23.

Matched tools are under:

```text
<sysroot>/lib/rustlib/<rustc-host>/bin/
```

The importer must use matching `llvm-dis`, `llvm-nm`, `llvm-size`, `llvm-objdump`, and `opt` tools.
It must convert bitcode to indexed text before removal of the matching toolchain.

## Evidence channels

No evidence channel describes a Rust instance completely.

| Channel | Useful facts | Main limit |
| --- | --- | --- |
| Rustc identity manifest | Concrete instances, raw symbols, and CGU placement. | It requires exact-version rustc-private libraries. |
| Mono statistics | Instance count and estimated aggregate cost by generic definition. | Estimates are compiler units, not object bytes. |
| MIR text | Rust control flow, types, and MIR inline scopes. | It covers local definitions and omits downstream bodies for dependency generics. |
| MIR pass dumps | Local transformation history. | A small crate produced 500 files, and dependency generic bodies remained absent. |
| Inline remarks | Successful and unsuccessful inline decisions, costs, and reasons. | Remarks cover selected LLVM passes only. |
| Inline debug metadata | Source locations that remain in final caller bodies. | Optimization can remove or move locations. |
| Type-size output | Size, alignment, variants, fields, closures, and async state. | The records belong to one compiler and target. |
| Phase timing | Time and memory for compiler phases. | The records describe a complete rustc invocation. |
| LLVM time trace | Pass cost associated with raw function symbols. | The small fixture produced 16,991 events and 3,379,453 bytes. |
| Dep-info | Reported source files and selected environment dependencies. | It omits undeclared macro and build-script inputs. |
| Rustc artifact messages | Exact paths for explicit compiler outputs. | Saved intermediate files do not receive these messages. |

The fixture produced 60 successful and 27 unsuccessful inline remarks. Empty remark files were
valid outputs and did not mean that capture failed.

## Identity findings

The following findings require the catalog to store identity facts separately:

- A public Rust path and its canonical LLVM path can name the same function differently.
- LLVM merged distinct type and constant instances into one body with several aliases.
- One mono item appeared in more than one CGU.
- MIR inlining removed local parent items before the final mono inventory.
- One module contained v0 and legacy symbol mangling at the same time.
- Legacy readable symbols omitted concrete generic arguments.
- Debug metadata connected exported Rust names to custom linkage names.
- Virtual-method tables appeared as anonymous globals without normal Rust item identities.
- LLVM sometimes added a generated identifier named `!guid` that connected symbols across ThinLTO
  stages.

Raw symbols provide exact identity only inside one compatible build. Cross-build matching must use
definition origin, Rust path, generic arguments, compiler, target, and configuration.

## Cargo behavior

### Freshness

An unchanged dependency build reported `fresh: true` and did not start the wrapper. A selected
target uses separate analysis flags and receives a separate Cargo fingerprint.

Cargo uses a fingerprint to decide whether an artifact is fresh. Wrapper flags do not reliably
change this fingerprint.

Use Cargo's normal target directory and dependency graph. Do not move or clean the target directory.
The selected target can compile again for analysis without rebuilding fresh dependencies.

### Wrappers

`RUSTC_WRAPPER` preserved the observed CGU identity. `RUSTC_WORKSPACE_WRAPPER` changed Cargo
artifact hashes. Optic uses the outer global wrapper and preserves both existing wrappers in their
original order.

The exact-version driver replaces rustc only for the selected invocation. Compiler probes and
dependencies pass through the original wrapper chain. A warm fixture compiled the selected target
once and reused its dependency artifact.

Cargo uses small rustc queries to inspect the compiler. These queries are compiler probes. Version,
sysroot, target, print requests, and Cargo's synthetic `___` crate must not receive evidence flags.

A cache wrapper can return success without the expected artifacts. In this case, Optic cannot prove
that rustc ran. Bypassing an existing wrapper needs an explicit option.

### Targets and modes

One Cargo command can contain host and target rustc invocations. The absence of `--target` means the
rustc host for that process.

One package can compile as a library, binary, test, benchmark, example, build script, or procedural
macro. Store Cargo mode and actual rustc flags independently.

Cargo artifact paths do not follow one stable directory layout. Use the paths from Cargo JSON and
the rustc arguments.

### Doctests

Rustdoc starts doctest compiler processes outside `RUSTC_WRAPPER`. The adapter needs rustdoc's
`--test-builder-wrapper` path.

Rustdoc can pass arguments through temporary response files and source through standard input. The
adapter must capture both before rustdoc removes them.

Cargo rejects `cargo test --doc --no-run` on the tested nightly. Rustdoc also started a test tool
with its own `--no-run` flag. Compile-only collection needs a recorded no-op `--test-runtool`.

A `compile_fail` doctest expects its child compiler to fail. Publish its diagnostics, but do not
publish reusable body evidence from that child.

## Source and security findings

Dep-info uses Makefile escaping. The fixture path `kernel/src/space name.rs` appeared as
`kernel/src/space\ name.rs`.

Downstream dep-info omitted the dependency source that defined a generic instance. Debug metadata
still referred to that source. Source capture must include all resolved packages.

Build-script Cargo events can contain clear-text `rustc-env` values. Persist sanitized structured
events instead of an unfiltered Cargo log.

Build scripts and procedural macros can read undeclared inputs. They also run with the user's
authority and can modify `.optic`. Analysis of an untrusted project requires an operating-system
sandbox.

## Sources

- [Cargo wrapper variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)
- [Cargo artifact messages][cargo-artifacts]
- [Cargo unit graph](https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph)
- [Cargo build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)
- [Rustc code-generation configuration](https://doc.rust-lang.org/rustc/codegen-options/index.html)
- [Rustc monomorphization](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html)
- [Rust symbol mangling](https://doc.rust-lang.org/rustc/symbol-mangling/index.html)
- [Rustdoc unstable features](https://doc.rust-lang.org/rustdoc/unstable-features.html)
- [LLVM language reference](https://llvm.org/docs/LangRef.html)
- [LLVM optimization remarks](https://llvm.org/docs/Remarks.html)

[cargo-artifacts]: https://doc.rust-lang.org/cargo/reference/external-tools.html#artifact-messages
