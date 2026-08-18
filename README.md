# Cargo Optic

Cargo Optic shows the Rust source and LLVM output for concrete compiler instances. It records this
evidence in the Cargo workspace, so later queries do not compile the same inputs again.

This prototype supports these compiler outputs:

- `llvm` is the saved LLVM IR after the optimization pipeline. This output is the default.
- `llvm-pre-opt` is the LLVM IR before the LLVM optimization pipeline.

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
ambiguous, the command prints a complete `show` command for each candidate. Copy one command to
request that result. The command keeps your `--source` and `--output` options.

```console
cargo +nightly optic show \
  --instance ins_01234567 \
  --source
```

The default command shows only optimized LLVM IR. Use `--output llvm-pre-opt` to show the
pre-optimization LLVM IR. The source is absent unless you add `--source`.

The default format is plain text. Add `--format json` to get a versioned JSON envelope.

Cargo Optic highlights interface text, Rust source, and LLVM IR when standard output is a terminal.
Use `--color always` to keep color in redirected output. Use `--color never` to disable color.

The `NO_COLOR` environment variable also disables automatic color. JSON output never contains ANSI
escape sequences.

## Capture and query separately

Use these commands when an agent or another program controls the workflow:

```console
cargo +nightly optic capture -p my-crate --lib --release --format json
cargo +nightly optic find --capture CAPTURE_ID_PREFIX my_crate::kernel --format json
cargo +nightly optic show --instance INSTANCE_ID_PREFIX --format json
cargo +nightly optic captures --format json
```

Omit `--format json` for an interactive workflow. Plain `find` output prints a complete `show`
command after each instance. You do not need to copy an ID into a new command.

Plain `capture` output prints `find` and `show` command templates for the new capture. Replace
`QUERY` with a definition path.

Plain output shows the shortest unique prefix for each capture and instance ID. JSON output keeps
the full IDs. You can use any longer unique prefix. Cargo Optic reports an error for a collision.

Use `--fresh` with `capture` or a build-based `show` command to create new evidence. This option
does not use a matching completed capture.

## Persistent state

Cargo Optic stores immutable captures in `.optic`. The SQLite catalog uses WAL mode. A file lock
serializes capture writers, but read-only queries can use completed captures in parallel.

There is no current capture and no session state. Each read-only command uses an explicit capture
or instance ID. An instance ID identifies its capture. Content-addressed blobs hold the evidence.

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
