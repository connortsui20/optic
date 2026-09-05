# Research fixtures

These fixtures reproduce the compiler observations in the research documents. They are diagnostic
tools, not production collector code. Read [`../core.md`](../core.md) for the required compiler
background.

The commands below preserve the original nightly experiments. They are not current `cargo optic`
usage instructions.

## Codegen workspace

[`codegen/`](codegen/) is a Cargo workspace without external crate dependencies. It covers:

- Type and const generic instances.
- Functions that remain separate and functions that move into their callers.
- Trait, inherent, async, closure, projection, and dynamic-dispatch code.
- LLVM aliases that result when LLVM merges identical functions.
- Duplicate short names, Unicode names, statics, and exported symbols.
- A build script, generated source, generated rustc environment, procedural macro, and a source
  path with a space.
- Binary, library, unit-test, integration-test, example, benchmark, and doctest modes.

Run the normal tests:

```bash
cd docs/research/fixtures/codegen
cargo +nightly test --workspace --all-targets
cargo +nightly test --doc -p optic-research-kernel
```

Capture the normal LLVM stages for one release build. Use an isolated temporary directory:

```bash
cd docs/research/fixtures/codegen
capture_dir=$(mktemp -d)
cargo +nightly rustc -p optic-research-app --release -- \
  -Csave-temps=yes \
  -Ztemps-dir="$capture_dir" \
  --emit=mir \
  -Zprint-mono-items=yes \
  -Zdump-mono-stats="$capture_dir/mono" \
  -Zdump-mono-stats-format=json \
  -Cremark=inline \
  -Zremark-dir="$capture_dir/remarks"
```

The exact nightly flag syntax can change. Record `rustc -vV` with each result.

Use a dedicated `CARGO_TARGET_DIR` for each configuration comparison. Otherwise, Cargo can skip
rustc and reuse a prior result. Do not find current files with a glob in an incremental output
directory. The directory retains files from old sessions.

The experiments also built the fixture for wasm:

```bash
cd docs/research/fixtures/codegen
rustup target add --toolchain nightly wasm32-unknown-unknown
cargo +nightly build --workspace --all-targets --target wasm32-unknown-unknown
```

This command still compiles build scripts and procedural macros for the host platform.

## Wrapper recorders

[`record-rustc.sh`](record-rustc.sh) appends wrapper arguments to `OPTIC_WRAPPER_LOG`. Use it to
inspect Cargo's wrapper order:

```bash
cd docs/research/fixtures/codegen
wrapper_log=$(mktemp)
OPTIC_WRAPPER_LOG="$wrapper_log" \
RUSTC_WRAPPER="$(pwd)/../record-rustc.sh" \
cargo +nightly check --workspace
```

Set both Cargo wrapper variables to the recorder to observe nesting. This experiment produces two
records for each compiler call.

[`capture-rustc-output.sh`](capture-rustc-output.sh) copies and forwards the child compiler's
standard output and standard error. It demonstrates that compiler evidence uses both streams. The
script uses shell process substitution. It does not support byte-safe paths, process supervision,
cancellation, or atomic manifests.

[`skip-doctest-run.sh`](skip-doctest-run.sh) is a diagnostic no-op test tool for rustdoc. It records
the executable argument and returns success without starting the doctest. It reproduces the
compile-only mechanism for the tested nightly:

```bash
cd docs/research/fixtures/codegen
run_log=$(mktemp)
OPTIC_DOCTEST_RUN_LOG="$run_log" \
RUSTDOCFLAGS="-Zunstable-options --test-runtool $(pwd)/../skip-doctest-run.sh" \
cargo +nightly test --doc -p optic-research-kernel
```

The production adapter must preserve platform-native argument bytes and the configuration of the
existing test tool. The shell recorder does not preserve them.

## Streaming IR indexer

[`ir-indexer/`](ir-indexer/) is a Rust prototype without external crate dependencies. It scans
textual LLVM IR with a bounded input buffer. It records 64-bit byte ranges. Run its tests:

```bash
cd docs/research/fixtures/ir-indexer
cargo +nightly test
```

Scan one textual module:

```bash
cargo +nightly run --release -- path/to/module.ll
```

The command reports counts for definitions, declarations, aliases, indirect-function symbols, and
globals. It also reports the largest function range.

The prototype proves that a bounded-memory scan works. It does not yet index multiline globals,
attributes, named types, metadata, call references, or every LLVM syntax form. Those cases remain in
[`../reference/test-matrix.md`](../reference/test-matrix.md).
