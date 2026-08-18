# Cargo Optic

Cargo Optic shows the Rust source and LLVM output for concrete compiler instances. It records this
evidence in the Cargo workspace, so later queries do not compile the same inputs again.

This prototype supports these compiler stages:

- `llvm-pre-optimization` is the LLVM IR before the LLVM optimization pipeline.
- `llvm-optimized` is the saved LLVM IR after the optimization pipeline.

The evidence is enriched output. Cargo Optic enables v0 symbol names and line-table debug
information. Therefore, the output does not exactly match a normal Cargo build.

## Install

Install a nightly toolchain and its matching LLVM tools:

```console
rustup toolchain install nightly --component llvm-tools
cargo install --path crates/cargo-optic
```

Cargo Optic uses the active compiler. Use `cargo +nightly optic` to select the installed nightly
toolchain.

## Inspect a function

Run `show` with the Cargo target options and a Rust definition path:

```console
cargo +nightly optic show my_crate::kernel -p my-crate --lib --release --source
```

Cargo Optic captures the selected target and finds its concrete compiler instances. If the query is
ambiguous, the command lists each candidate and stops. Use the instance ID to request one result:

```console
cargo +nightly optic show \
  --capture cap_0123456789abcdef0123456789abcdef \
  --instance ins_0123456789abcdef0123456789abcdef \
  --source
```

The default output is plain text. The source is absent unless you add `--source`. Add
`--format json` to get a versioned JSON envelope.

## Capture and query separately

Use these commands when an agent or another program controls the workflow:

```console
cargo +nightly optic capture -p my-crate --lib --release --format json
cargo +nightly optic find --capture CAPTURE_ID my_crate::kernel --format json
cargo +nightly optic show --capture CAPTURE_ID --instance INSTANCE_ID --format json
cargo +nightly optic captures --format json
```

Use `--fresh` with `capture` or a build-based `show` command to create new evidence. This option
does not use a matching completed capture.

## Persistent state

Cargo Optic stores immutable captures in `.optic`. The SQLite catalog uses WAL mode. A file lock
serializes capture writers, but read-only queries can use completed captures in parallel.

There is no current capture and no session state. Each read-only command uses an explicit capture
ID. Content-addressed blobs hold source, bitcode, and textual LLVM modules.

The cache key includes these inputs:

- The Cargo target options.
- The rustc commit.
- The Cargo metadata.
- Cargo manifests, the lock file, and Cargo configuration files.
- The contents of local Rust source files.
- Compiler and Cargo environment variables.

If a build script reads an undeclared input, use `--fresh`. The first MVP does not find all
external build-script inputs.

## Cargo artifact reuse

Cargo Optic uses `cargo rustc`, the normal target directory, and the normal Cargo dependency graph.
Cargo can reuse dependency artifacts from normal commands.

The analysis flags apply only to the selected target. If no normal linked artifact exists, the next
normal Cargo command compiles that target. Existing normal artifacts remain available.

## MVP limits

- Cargo Optic requires nightly rustc and the matching `llvm-tools` component.
- The prototype records source from workspace packages and local path dependencies.
- The prototype records one selected library, binary, benchmark, or example target.
- The prototype does not record MIR, assembly, object files, or LTO transition stages.
- Source lookup uses Rust syntax and a path score. It omits source when the best match is ambiguous.
- The prototype is verified on Apple silicon macOS.

The unpublished `cargo-ir` crate owns the compiler and LLVM boundary. `cargo-optic` owns the
persistent store and the user interface.
