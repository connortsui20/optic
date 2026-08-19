# Cargo Optic

Cargo Optic shows Rust source and LLVM output for concrete compiler instances. It stores immutable
evidence in the Cargo workspace. Cargo decides when the selected target is fresh.

This prototype supports these compiler outputs:

- `llvm` is the saved LLVM IR after the optimization pipeline. This output is the default.
- `llvm-pre-opt` is the LLVM IR before the LLVM optimization pipeline.

The default `faithful` profile preserves the code-generation settings of the selected target. It
adds only the arguments that save compiler evidence. The saved temporary files still change the
Cargo fingerprint.

Use `--evidence-profile enriched` to add v0 symbol names and line-table debug information. Use
`--evidence-profile experiment` with repeated `--rustc-arg` options for a code-generation
experiment.

## Install

Install a nightly toolchain and its matching compiler and LLVM libraries:

```console
rustup toolchain install nightly --component llvm-tools --component rustc-dev
cargo +stable install --locked --path crates/cargo-optic
```

Cargo Optic uses the active compiler. Use `cargo +nightly optic` to select the installed nightly
toolchain.

## Try the included example

Run these commands from the Optic repository. They work in Fish, Bash, and Zsh.

```console
cd crates/cargo-optic/tests/fixtures/generic
cargo +nightly optic show optic_mvp_kernel::outlined_sum -p optic-mvp-app --bin optic-mvp-app --release --source
```

The example creates `u32` and `u64` instances of the same generic function. Cargo Optic lists both
instances and prints a complete `show` command for each one. Copy either `show` command.

See the [complete fixture guide](crates/cargo-optic/tests/fixtures/generic/example.md) to compare
evidence before and after a source change.

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
cargo +nightly optic inspect --capture CAPTURE_ID_PREFIX --format json
```

Omit `--format json` for an interactive workflow. Plain `find` output prints a complete `show`
command after each instance. You do not need to copy an ID into a new command.

Plain `capture` output prints `find` and `show` command templates for the new capture. Replace
`QUERY` with a definition path.

Plain output shows at least 12 hexadecimal characters for each ID. Color highlights the shortest
unique prefix and dims the remaining characters. JSON output keeps the full IDs.

Each displayed ID is a valid prefix. Cargo Optic reports an error if a shorter prefix matches more
than one stored ID.

Use `--fresh` with `capture` or a build-based `show` command to create new evidence. This option
uses a unique Cargo fingerprint and invokes rustc for the selected target.

The JSON transport version is 2. Instance results report definitions, declarations, and aliases
for each LLVM stage. A result does not use one combined `has_body` value.

## Inspect and compare evidence

Use `inspect` to show the request, compiler commands, wrappers, environment, and artifact stages:

```console
cargo +nightly optic inspect --capture CAPTURE_ID_PREFIX
```

Use `compare` to compare compact LLVM structure for two exact instances:

```console
cargo +nightly optic compare \
  --before OLD_INSTANCE_ID \
  --after NEW_INSTANCE_ID
```

The comparison reports body bytes, instruction-like lines, vector lines, calls, and safety-check
symbols. It also reports incompatible compiler or Cargo dimensions. These counts are structural
LLVM summaries, not performance measurements.

## Manage stored evidence

Use these commands to inspect and manage the store:

```console
cargo +nightly optic status
cargo +nightly optic verify
cargo +nightly optic remove --capture CAPTURE_ID_PREFIX
cargo +nightly optic gc
```

The `remove` command removes one catalog capture. Shared blobs remain until `gc` removes all
unreferenced blobs. The `verify` command reads each referenced blob and checks its BLAKE3 digest.

Run this command from the Cargo workspace that you want to clean:

```console
cargo +nightly optic clean
```

The command removes only `.optic` in the workspace. It does not remove the Cargo `target`
directory. The command succeeds when the Optic cache does not exist.

## Persistent state

Cargo Optic stores immutable captures in `.optic`. The SQLite catalog uses WAL mode. A file lock
serializes capture writers, but read-only queries can use completed captures in parallel.

There is no current capture and no session state. Each read-only command uses an explicit capture
or instance ID. An instance ID identifies its capture. Content-addressed blobs hold the evidence.

The current store schema is version 5. Cargo Optic rejects older stores. Run `cargo optic clean`
once to replace an older prototype store.

Cargo Optic asks Cargo to evaluate the selected target before it reuses a capture. This design
includes Cargo-tracked build-script inputs, `include_bytes!` files, and compiler environment
inputs. Optic does not use a source-file digest as a substitute for Cargo freshness.

Optic uses a stable analysis fingerprint for one compiler, target, profile, feature set, and
compiler environment. If Cargo reports the target as fresh, Optic reuses the matching verified
capture. If no matching capture exists, Optic asks you to repeat the command with `--fresh`.

Cargo cannot track a build-script input that the script does not declare. If a script has an
undeclared input, use `--fresh` after that input changes.

## Cargo artifact reuse

Cargo Optic uses `cargo rustc`, the normal target directory, and the normal Cargo dependency graph.
Cargo can reuse dependency artifacts from normal commands.

Cargo Optic installs an outer global compiler wrapper. This wrapper preserves existing global and
workspace wrappers. It passes dependency compilations and compiler probes through without changes.

The evidence arguments and the compiler identity driver apply only to the selected target. Cargo
Optic records identities before code generation and continues the same compilation.

The driver is a small internal program. Cargo Optic builds it once for each rustc commit and source
revision. It stores the driver below `$CARGO_HOME/optic/drivers`.

The faithful profile permits the normal link step. Existing normal artifacts remain available.

## MVP limits

- Cargo Optic requires nightly rustc and the matching `llvm-tools` and `rustc-dev` components.
- The prototype records source from workspace packages and local path dependencies.
- The prototype records one selected library, binary, benchmark, or example target.
- The prototype does not record MIR, assembly, object files, or compiler optimization remarks.
- The store retains exact compiler stage names. The user interface shows only supported LLVM
  stages.
- If ThinLTO artifacts exist, optimized output uses `thin-lto-after-pm` instead of the earlier
  `rcgu` artifact.
- Source lookup uses exact rustc spans when they match captured local source. Syntax scoring is a
  fallback for identities that do not have a span.
- The prototype does not navigate inline occurrences to their enclosing optimized bodies.
- The prototype is verified on Apple silicon macOS.

### Instance-to-body identity

Cargo Optic uses a small rustc driver to collect each concrete function and raw LLVM symbol. Each
instance retains its source definition and codegen-unit placements. Duplicate symbols remain
separate instance records.

The store keeps instances, definitions, bodies, declarations, and aliases as separate records. An
exact raw-symbol relationship connects an instance to zero or more bodies. Display paths do not
control this relationship.

LLVM can remove, clone, or rename a body during optimization. Cargo Optic does not infer a
relationship from similar text. If the exact symbol is absent, the selected output has no
standalone body. A pre-optimization body can still be available.

The unpublished `cargo-ir` crate owns the compiler and LLVM boundary. `cargo-optic` owns the
persistent store and the user interface.
