# Cargo Optic

Cargo Optic shows Rust source and LLVM output for concrete compiler instances. It stores immutable
evidence in the Cargo workspace. Cargo decides when the selected target is fresh.

This prototype supports these compiler outputs:

- `llvm` is the saved LLVM IR after the optimization pipeline. This output is the default.
- `llvm-pre-opt` is the LLVM IR before the LLVM optimization pipeline.
- `remarks` contains the saved LLVM optimization remarks for one concrete compiler instance.

The default `faithful` profile preserves the code-generation settings of the selected target. It
adds only the arguments that save compiler evidence. The saved temporary files still change the
Cargo fingerprint.

Use `--evidence-profile enriched` to add v0 symbol names and line-table debug information. Use
`--evidence-profile experiment` with repeated `--rustc-arg` options for a code-generation
experiment.

## Install

Install Cargo Optic from this repository:

```console
cargo install --locked --path crates/cargo-optic
```

The repository toolchain file installs `rustc-dev` and `llvm-tools`. For a different workspace, add
these components to the toolchain that the workspace selects:

```console
rustup component add rustc-dev llvm-tools
```

Use `cargo optic` without a toolchain prefix. Cargo Optic uses the Cargo and rustc that the
workspace selects through the normal Rust configuration.

Cargo Optic accepts the workspace rustc without a release-channel restriction. Its internal
unstable-access policy is limited to Cargo configuration discovery, exact-version driver
compilation, and selected-target compilation.

Cargo Optic does not add unstable access to dependencies, build scripts, or compiler probes. Do not
set `RUSTC_BOOTSTRAP` for Cargo Optic.

## Try the included example

Run these commands from the Optic repository. They work in Fish, Bash, and Zsh.

```console
cd crates/cargo-optic/tests/fixtures/generic
cargo optic show optic_mvp_kernel::outlined_sum -p optic-mvp-app --bin optic-mvp-app --release --source
```

The example creates `u32` and `u64` instances of the same generic function. Cargo Optic lists both
instances and prints a complete `show` command for each one. Copy either `show` command.

See the [complete fixture guide](crates/cargo-optic/tests/fixtures/generic/example.md) to compare
evidence before and after a source change.

## Inspect a function

Run `show` with the Cargo target options and a Rust definition path:

```console
cargo optic show my_crate::kernel -p my-crate --lib --release --source
```

Cargo Optic captures the selected target and finds its concrete compiler instances. If the query is
ambiguous, the command prints a complete `show` command for each candidate. Copy one command to
request that result. The command keeps your `--source` and `--output` options.

```console
cargo optic show \
  --instance ins_01234567 \
  --source
```

The default command shows only optimized LLVM IR. Use `--output llvm-pre-opt` to show the
pre-optimization LLVM IR. Use `--output remarks` to capture and show optimization remarks. The
source is absent unless you add `--source`.

The default format is plain text. Add `--format jsonl` to get versioned JSON Lines events.

In text mode, Cargo Optic writes Cargo progress and compiler diagnostics to standard error. It
writes source, compiler output, and the final result to standard output as they arrive.

In JSON Lines mode, Cargo Optic writes typed progress, diagnostic, data, and terminal events to
standard output. Each line is one complete JSON value. A successful stream ends with one
`complete` event. A failed stream ends with one `error` event.

Cargo Optic highlights interface text, Rust source, and LLVM IR when standard output is a terminal.
Use `--color always` to keep color in redirected output. Use `--color never` to disable color.

The `NO_COLOR` environment variable also disables automatic color. JSON Lines output never
contains ANSI escape sequences.

## Capture and query separately

Use these commands when an agent or another program controls the workflow:

```console
cargo optic capture -p my-crate --lib --release --format jsonl
cargo optic find --capture CAPTURE_ID_PREFIX my_crate::kernel --format jsonl
cargo optic show --instance INSTANCE_ID_PREFIX --format jsonl
cargo optic captures --format jsonl
cargo optic inspect --capture CAPTURE_ID_PREFIX --format jsonl
```

Omit `--format jsonl` for an interactive workflow. Plain `find` output prints a complete `show`
command after each instance. You do not need to copy an ID into a new command.

Plain `capture` output prints `find` and `show` command templates for the new capture. Replace
`QUERY` with a definition path.

Use `find --crate NAME` or `find --definition PATH` to restrict a large result set. Use
`--available llvm` or `--available llvm-pre-opt` to require a standalone body. The default result
limit is 50 and the maximum is 500.

Lookup first checks exact definition paths, display names, and compiler symbols. A fallback lookup
matches a case-sensitive literal substring and requires at least three Unicode characters. Results
report truncation. JSON Lines output also reports the match kind, full compiler symbol, and a
stable identity fingerprint.

Plain output shows at least 12 hexadecimal characters for each ID. Color highlights the shortest
unique prefix and dims the remaining characters. JSON Lines output keeps the full IDs.

Each displayed ID is a valid prefix. Cargo Optic reports an error if a shorter prefix matches more
than one stored ID.

Use `--fresh` with `capture` or a build-based `show` command to request new evidence. Cargo Optic
first tries to resume matching post-compilation evidence. Otherwise, `--fresh` uses a unique Cargo
fingerprint and invokes rustc for the selected target.

The JSON Lines transport version is 1. Every event has a version, sequence number, command, event
name, and typed data. Sequence numbers increase during one process. Text payloads use chunks of at
most 64 KiB. A diagnostic uses base64 when its bytes are not valid UTF-8.

Instance results report definitions, declarations, and aliases for each LLVM stage. A result does
not use one combined `has_body` value.

If compilation succeeds but ingestion fails, Cargo Optic retains validated staging evidence. The
next matching request runs Cargo's freshness check. If fresh, it resumes ingestion without another
selected-target compilation. `status` reports the pending capture count and total retained bytes.
Use `pending` to list retained runs, `pending inspect ID` to inspect one, and `pending remove ID` to
discard one. `clean` removes all pending evidence with the rest of the store.

## Capture optimization remarks

A build-based `show` command captures remarks automatically when you select them:

```console
cargo optic show my_crate::kernel \
  -p my-crate --lib --release \
  --output remarks
```

Use `capture --remarks` when you want to capture first and query later. Capture-wide states
distinguish remarks that were not requested, a completed capture with no records, and a capture
with records. A selected instance or filter can still have no matching remarks.

Use `--kind KIND`, `--pass NAME`, and `--limit NUMBER` to filter remark output. These options apply
only to `--output remarks`. Use the `enriched` evidence profile when source locations are needed.

## Inspect and compare evidence

Use `inspect` to show the request, compiler commands, wrappers, environment, and artifact stages:

```console
cargo optic inspect --capture CAPTURE_ID_PREFIX
```

The result includes the Cargo and rustc paths. It also includes the rustc release, commit, host,
LLVM version, sysroot, and matching `llvm-dis`. The bootstrap policy lists the only scopes that are
authorized to use unstable access. An authorized cached step does not necessarily run.

Use `compare` to compare compact LLVM structure for two exact instances:

```console
cargo optic compare \
  --before OLD_INSTANCE_ID \
  --after NEW_INSTANCE_ID
```

The comparison reports body bytes, instruction-like lines, vector lines, call categories, and
safety-check symbols. Call categories separate runtime calls, indirect calls, memory intrinsics,
and compiler metadata intrinsics.

The compatibility result checks the compiler commit, host, environment, wrapper chain, and
effective rustc arguments. It ignores Optic arguments that only collect evidence. The structural
counts are not performance measurements.

Use `--optic-dir PATH` to read completed or pending evidence from another workspace without Cargo
discovery or Optic-state mutation. The path must name an existing `.optic` directory with the same
store schema. This is useful for another worktree of the same project.

```console
cargo optic --optic-dir ../other-worktree/.optic captures
cargo optic --optic-dir ../other-worktree/.optic show --instance INSTANCE_ID
```

A comparison can select each store independently:

```console
cargo optic compare \
  --before OLD_INSTANCE_ID --before-optic-dir ../old/.optic \
  --after NEW_INSTANCE_ID --after-optic-dir ../new/.optic
```

## Manage stored evidence

Use these commands to inspect and manage the store:

```console
cargo optic status
cargo optic pending
cargo optic verify
cargo optic remove --capture CAPTURE_ID_PREFIX
cargo optic gc
```

`status` separates referenced and reclaimable blob bytes. It also reports total retained bytes,
the effective limit, available filesystem space, and the required free-space reserve.

The default retained limit is the smaller of 32 GiB and 25 percent of filesystem capacity. The
default free-space reserve is the smaller of 10 GiB and 5 percent of capacity. Set a workspace
limit in `.optic/config.toml`:

```toml
[store]
max_bytes = "64GiB"
```

Use `--max-store-bytes 8GiB` on `capture` or a build-based `show` command for a one-command
override. Cargo Optic checks the policy before compilation, while it publishes blobs, and at
bounded intervals during ingestion. These admission checks do not reserve space or impose a hard
filesystem quota. Compiler output and the temporary file for one blob can cross a configured limit
before the next check rejects the capture. A failed capture can therefore leave pending evidence or
unreferenced blobs above the limit until you remove the pending run, run `gc`, or run `clean`.
Completed evidence remains readable when a store is over its limit.

The `remove` command removes one catalog capture. Shared blobs remain until `gc` removes all
unreferenced blobs. The `verify` command reads each referenced blob and checks its BLAKE3 digest.

Run this command from the Cargo workspace that you want to clean:

```console
cargo optic clean
```

The command removes only `.optic/store`. It preserves `.optic/locks` and other entries below
`.optic`. It does not remove the Cargo `target` directory. The command succeeds when `.optic/store`
does not exist.

The `.optic` root is reserved for durable workspace state. `clean` preserves `.optic/config.toml`.
Cargo Optic does not create `.optic.lock` in the workspace root.

## Persistent state

Cargo Optic stores immutable captures in `.optic/store`. The SQLite catalog uses WAL mode. A file
lock serializes capture writers, but read-only queries can use completed captures in parallel.

Cargo Optic stores its operation and data locks in `.optic/locks`. These locks remain after
`cargo optic clean` so that `clean` can coordinate with other commands.

There is no current capture and no session state. Each read-only command uses an explicit capture
or instance ID. An instance ID identifies its capture. Content-addressed blobs hold the evidence.
Blob names remain the BLAKE3 digest of their uncompressed content. Schema 10 stores every blob as
one zstd frame at zstd's default compression level and decompresses logical byte ranges
transparently.

Cache lookup opens every referenced blob and checks its zstd frame header before Cargo evaluates
freshness. It does not decompress and hash complete blobs. Actual evidence reads and `cargo optic
verify` perform the complete logical BLAKE3 verification.

The current store schema is version 10. Cargo Optic rejects older stores. Run `cargo optic clean`
once to replace an older prototype store.

This prototype also uses JSON Lines transport version 1, evidence request version 5, pending marker
version 1, and compiler manifest protocol version 3. These formats have no compatibility promise.

Cargo Optic asks Cargo to evaluate the selected target before it reuses a capture. This design
includes Cargo-tracked build-script inputs, `include_bytes!` files, and compiler environment
inputs. Optic does not use a source-file digest as a substitute for Cargo freshness.

Optic stores the analysis fingerprint that produced each cached capture. A normal command uses that
exact fingerprint when it asks Cargo to check freshness. A fresh command creates and stores a new
fingerprint. The next normal command can therefore reuse the fresh capture.

Cargo cannot track a build-script input that the script does not declare. If a script has an
undeclared input, use `--fresh` after that input changes.

## Cargo artifact reuse

Cargo Optic uses `cargo rustc`, the normal target directory, and the normal Cargo dependency graph.
Cargo can reuse dependency artifacts from normal commands.

Cargo Optic installs an outer global compiler wrapper. This wrapper preserves existing global and
workspace wrappers. It passes dependency compilations and compiler probes through without changes.

Cargo Optic clears an ambient `RUSTC_BOOTSTRAP` value from Cargo metadata and compilation. It
restores `RUSTC_BOOTSTRAP=1` only for the selected target.

The evidence arguments and the compiler identity driver apply only to the selected target. Cargo
Optic records identities before code generation and continues the same compilation.

Cargo Optic uses the Cargo process that invoked the external subcommand. It resolves the effective
rustc and compiler wrappers from the same workspace configuration. Rustc inspection and driver
compilation also run from that workspace.

Cargo output uses bounded streaming. One Cargo JSON message and the retained failure tail can each
use at most 1 MiB. Cargo Optic drains the child process before it reports a message-limit error.
If an application consumer stops a capture stream, Cargo Optic stops and reaps Cargo and its
compiler descendants.

Evidence ingestion reads compiler identities, modules, remarks, and source files incrementally.
It writes them to a private SQLite staging catalog below the pending capture. The completed capture
becomes visible only after one successful catalog transaction.

The selected rustc requires matching `rustc-dev` and `llvm-tools` components. Cargo Optic reports a
specific error when one of these components is absent. It does not install components.

The driver is a small internal program. Its cache identity includes the compiler host, commit,
canonical sysroot digest, driver source revision, and protocol. Cargo Optic stores the driver below
`$CARGO_HOME/optic/drivers`.

The faithful profile permits the normal link step. Existing normal artifacts remain available.

## MVP limits

- Cargo Optic requires matching `llvm-tools` and `rustc-dev` components for the selected rustc.
- The prototype records source from workspace packages and feature-selected local path
  dependencies.
- The prototype records one selected library, binary, benchmark, or example target.
- The prototype does not record MIR, assembly, or object files.
- The store retains exact compiler stage names. The user interface shows only supported LLVM
  stages.
- If ThinLTO artifacts exist, optimized output uses `thin-lto-after-pm` instead of the earlier
  `rcgu` artifact.
- Source lookup requires an exact rustc definition span and canonical local source path. It returns
  no source when either identity is unavailable.
- The prototype does not navigate inline occurrences to their enclosing optimized bodies.
- The rustc identity driver requires UTF-8 compiler arguments. It returns an error for a non-UTF-8
  argument.
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
