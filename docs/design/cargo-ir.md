# `cargo-ir` spike results

This document preserves the original compiler experiments. Its exact commands and toolchain
versions are historical evidence, not the current product contract.

`cargo-ir` is the unpublished compiler-evidence library used by `cargo-optic`. It runs Cargo,
collects LLVM evidence, records compiler identities, and parses optimization remarks.

The library does not provide a command or own persistent state. `cargo-optic` is the only product
and owns the CLI and evidence store.

The current product uses the stable, beta, or nightly rustc that the workspace selects. Cargo Optic
uses scoped internal unstable access for the exact operations that require it.

These results informed that capture path. The exact nightly commands below remain useful for
reproducing the original observations.

The small fixture used a library crate and a downstream binary. The binary instantiated
`inlined_compare::<u64, 8>`, `inlined_compare::<u32, 4>`, and matching outlined functions.

The local toolchains were:

- Stable Rust 1.97.1 with LLVM 22.1.6.
- Nightly Rust 1.100.0-nightly from 2026-08-16 with LLVM 23.1.0.
- The `aarch64-apple-darwin` host and the installed `wasm32-unknown-unknown` target.

## RESOLVED: Inlining

Optimized IR contained both outlined bodies. It did not contain a standalone body for either
`inlined_compare` instantiation.

`-C no-prepopulate-passes` did not recover the complete inlined function. It exposed many internal
iterator bodies and closures, but the parent function was still absent. Pre-pass IR is useful as a
separate view, not as a replacement body.

`-C debuginfo=line-tables-only` retained the concrete inlined instantiation in `DISubprogram`
metadata. The caller's instructions reached it through `DILocation` and `inlinedAt` chains. This
supports a best-effort "contribution in caller" view.

The tool must report the missing body directly. It must not pretend that attributed caller
instructions form the original function. An explicit `#[inline(never)]` experiment can provide a
perturbed view, but the tool must label that result as perturbed.

## RESOLVED: v0 mangling

`rustfilt`, which uses `rustc-demangle`, produced these names from the two emitted symbols:

```text
codegen_spike_kernel::outlined_compare::<u32, 4>
codegen_spike_kernel::outlined_compare::<u64, 8>
```

The v0 format reversibly encodes type and const arguments. It also supports lifetimes and unnameable
entities. The format can use `_` placeholders when a generic argument does not affect the
monomorphized item.

The raw mangled symbol must remain the identity within one build. The demangled path is suitable for
display and lookup. It is not a stable cross-build identifier because the rendered form is not
standardized and the encoding is not a Rust ABI.

## RESOLVED: Compiler-instance mapping

The first spike used `-Z print-mono-items=yes`. This flag reports display text, CGU placement, and
linkage. It does not report the raw symbol for the compiler instance.

With four CGUs, nightly printed the demangled mono item, the exact CGU, and its linkage. For
example:

```text
MONO_ITEM fn codegen_spike_kernel::outlined_compare::<u64, 8> \
    @@ codegen_spike_app.f1582859b3a597d2-cgu.1[External]
```

This limitation caused a real lookup error in Vortex PR #9398. Rustc reported
`layouts::...::BloomPartial::add_hash`. Its LLVM body used the canonical `vortex_layout` module
path. Text normalization did not give a reliable relationship.

The current capture path uses an isolated rustc driver. The driver calls rustc's monomorphization
query in `after_analysis`. It records each function's definition path, concrete display name, raw
symbol, and codegen-unit placements. Then rustc continues the same compilation.

Optic disables existing global and workspace wrappers, then installs its outer wrapper. It prints a
warning when it disables a wrapper. The outer wrapper passes compiler probes and dependency
compilations through without changes. It inserts the driver only when the rustc arguments contain
the selected capture's private marker.

The driver receives the real rustc path and the original arguments. It runs analysis, writes the
identity manifest after a successful compilation, and permits normal code generation. This process
does not compile the selected target twice.

The embedded driver source has no Cargo dependencies. The early compiler slices compile this helper
for each capture. The cache slice reuses a compatible driver. Concurrent provisioning remains
future work.

The selected rustc must contain matching `rustc-dev` and `llvm-tools` components. Optic returns an
actionable error when either component is absent. It does not fall back to display-name matching.

The identity manifest has a versioned binary protocol. It contains a magic header, protocol
version, and length-prefixed item fields. The reader rejects malformed and wrong-version manifests.

The LLVM scanner indexes each function by its raw symbol. The adapter joins a compiler instance to a
body only when their raw symbols are equal. The store keeps display names for lookup and output, not
for evidence relationships.

The Vortex check found the exact optimized `add_hash` body. Apple silicon LLVM used two
`<4 x i32>` updates for its eight lanes. The exact pre-optimization `make_mask` body was also
available. LLVM inlined this body and removed its standalone optimized form.

LLVM can create a clone, merge bodies, or rename a symbol after rustc creates it. The adapter does
not connect these results by similar display text or `.llvm.N` suffixes. A future LLVM adapter can
record transformation lineage when a concrete query requires it.

## RESOLVED: Debug-info attribution

Line-table debug information preserved source files, lines, concrete generic arguments, and inline
call chains in the fixture. The optimized instructions often pointed first to an iterator function
in `core`. Their `inlinedAt` chains led back through the generic kernel and its call site.

This metadata is sufficient for best-effort source and inline-origin annotations. It is not an exact
partition of optimized instructions by source function. LLVM can merge, move, or remove instructions
and their locations during optimization.

Source attribution must remain an annotation. It must not define function identity or the bounds of
an extracted body.

## RESOLVED: Cross-target experiment

Stable Rust emitted LLVM IR for `wasm32-unknown-unknown`. The output had a different target triple,
data layout, pointer width, return types, and vectorization behavior from the host output.

`cargo rustc -- --emit=llvm-ir` still asked `rustc` to link the binary. A failing linker caused
Cargo to fail, although the `.ll` file remained on disk. A no-op linker made the build succeed and
left the requested IR.

Nightly Rust offered `-Z no-link`, which worked with a deliberately failing linker. This experiment
proved that no-link capture was possible.

The current faithful profile permits the normal link step. Cross-target support still needs target
and linker contract tests.

Textual comparison across targets is not meaningful by default. The tool can compare function
presence and source identity, but target-specific IR needs a clear warning.

## RESOLVED: Optimization remarks

Stable Rust accepts `-C remark=loop-vectorize`. The fixture produced all three useful classes:

- A passed remark reported vector widths of four lanes for `u32` and two lanes for `u64`.
- Analysis remarks included the reasons that other loops did not vectorize.
- Missed remarks reported the rejected loops.

Cargo's JSON message format wrapped each remark as a `compiler-message`. The structured `spans`
array was empty, so the source location remained part of the message text.

`-Z remark-dir` writes structured optimization remarks to a directory. Cargo Optic uses this output
for the selected target through its scoped unstable-access policy.

The collector parses bounded YAML records and retains the raw files. It distinguishes omitted,
captured-empty, and captured-with-records states.

## RESOLVED: Scale

The scale fixture was the Vortex `compare` benchmark at commit
`8946968faa68455499899e19d7968843d7b07f28`. The worktree was clean. Both builds used the bench
profile and line-table debug information for the final target.

| Build           | IR files |      Lines |         Bytes | Build time |
| --------------- | -------: | ---------: | ------------: | ---------: |
| 16 CGUs, no LTO |       16 |    113,299 |    11,935,374 |     1m 49s |
| 1 CGU, fat LTO  |        1 | 19,705,945 | 1,717,548,295 |     5m 53s |

The fat-LTO module contained 11,066 function definitions. This result rules out an owned parse of
the complete module.

The index must scan or memory-map raw artifacts and store byte ranges for each body. A collection
record can keep raw IR, a compact index, remarks, and build facts as separate files. Loading one
body must not load the full module.

Capture records live under `.optic/store` in the Cargo workspace. A bundled SQLite catalog stores
metadata and byte ranges. Content-addressed files store raw artifacts, remarks, and source
snapshots.

The prototype has no pins, automatic eviction, or store-size budget. Explicit commands remove
captures and collect unreferenced blobs.

The `cargo-ir` library returns an evidence bundle with typed stages and body byte ranges. It does
not expose database tables or persistent artifact identifiers. `cargo-optic` stores this evidence
and uses the library in-process.

<details>
<summary>Commands used for the main checks</summary>

```bash
cargo +stable rustc -p codegen-spike-app --release -- \
  -C symbol-mangling-version=v0 \
  -C debuginfo=line-tables-only \
  -C remark=loop-vectorize \
  --emit=llvm-ir

cargo +nightly rustc -p codegen-spike-app --release -- \
  -C codegen-units=4 \
  -Z print-mono-items=yes \
  --emit=llvm-ir

cargo rustc -p vortex-array --bench compare --profile bench -- \
  -C symbol-mangling-version=v0 \
  -C debuginfo=line-tables-only \
  --emit=llvm-ir

CARGO_PROFILE_BENCH_CODEGEN_UNITS=1 \
CARGO_PROFILE_BENCH_LTO=fat \
cargo rustc -p vortex-array --bench compare --profile bench -- \
  -C symbol-mangling-version=v0 \
  -C debuginfo=line-tables-only \
  --emit=llvm-ir
```

</details>

## Sources

- The [v0 symbol format](https://doc.rust-lang.org/stable/rustc/symbol-mangling/v0.html) defines
  generic arguments, placeholders, and the format's stability limits.
- The [rustc codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html) define
  `remark` and `no-prepopulate-passes`.
- The [external rustc driver guide][driver] defines the `rustc-dev` and `llvm-tools` requirements.
- The Rust Unstable Book documents [`print_mono_items`][mono].
- The Rust Unstable Book documents [`remark_dir`][remarks].
- The Rust Unstable Book documents [`no_link`][no-link].
- LLVM's [source-level debugging documentation](https://llvm.org/docs/SourceLevelDebugging.html)
  describes how optimization changes debug information.

[mono]: https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/print-mono-items.html
[remarks]: https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/remark-dir.html
[no-link]: https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/no-link.html
[driver]: https://rustc-dev-guide.rust-lang.org/rustc-driver/external-rustc-drivers.html
