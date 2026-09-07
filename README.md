# Cargo Optic

Cargo Optic captures concrete Rust instances from a Cargo target. It finds instances and shows their
captured source or optimized LLVM output. It is an experimental tool with versioned prototype data,
not a stable compiler interface.

## Install from the checkout

The supported compiler is Rust 1.98.1 on Linux or macOS. Runtime collection needs the matching
`rustc-dev` and `llvm-tools` components.

```console
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev --component llvm-tools
cargo +1.98.1 install --path crates/cargo-optic --locked
```

Run the tool in the workspace to inspect. Use the supported toolchain for that workspace.

## Capture and find

Select one package, target, and profile explicitly:

```console
cargo +1.98.1 optic capture -p my-crate --lib --release
cargo +1.98.1 optic list-captures
cargo +1.98.1 optic find --capture CAPTURE_REF kernel
```

Copy `CAPTURE_REF` from the capture output. Find prefers exact definition names, concrete instance
names, and raw symbols. Otherwise, it uses a case-sensitive literal substring. Results have a
deterministic order and a default limit of 20.

Other target selectors are `--bin NAME`, `--example NAME`, and `--bench NAME`. Use `--profile NAME`
for a custom profile. Cargo feature options are `--features`, `--all-features`, and
`--no-default-features`.

## Reuse and fresh analysis

Repeat the same capture command to let Cargo verify freshness. A fresh result prints `Reused` and
returns the original capture ID and completion time. It does not compile the selected target or
driver again, copy evidence, or add another capture.

Use `--fresh` to force selected-target analysis:

```console
cargo +1.98.1 optic capture -p my-crate --lib --release --fresh
```

Forced analysis retains compatible driver and dependency caches. Changes to Cargo-tracked inputs
also produce a new capture. Cargo metadata locates packages and targets, but Cargo's build freshness
decision determines whether evidence can be reused.

Completed captures live in `.optic/store`. Compatible drivers live under the Cargo home in
`optic/drivers`. Each real collection adds a capture and can leave another selected-target artifact.
There is no automatic retention or cleanup policy.

## Read captured evidence

Find prints an instance reference scoped to one immutable capture. Use that reference explicitly:

```console
cargo +1.98.1 optic show --instance INSTANCE_REF --output source
cargo +1.98.1 optic show --instance INSTANCE_REF --output llvm
```

Source output comes from the compiler's captured text, not the current checkout. LLVM output uses
exact compiler symbols and the captured optimization stage. It is a function excerpt, not a complete
independently assemblable LLVM module.

Evidence text goes to stdout. Diagnostics go to stderr. Unavailable evidence produces an explanation
and a failing exit status. It does not trigger another capture.

## Limitations

- The runtime driver supports the pinned compiler, not arbitrary Rust releases or custom compilers.
- Configured compiler wrappers are disabled with a warning. Wrapper composition is not supported.
- Captures require one writer per workspace and serialized driver provisioning within one Cargo home.
- Source capture supports local definitions with exact loaded spans in the selected package root.
- External source definitions, unsupported expansions, and synthetic spans can be unavailable.
- Source snapshots contain rustc-normalized UTF-8, including its BOM and CRLF normalization.
- Optimized LLVM initially supports nonincremental no-LTO and local ThinLTO configurations.
- Other LLVM configurations retain core capture/search with an explicit unavailable-evidence warning.
- Missing exact LLVM symbols do not prove that optimization eliminated the corresponding functions.
- Listing and evidence reads can run Cargo metadata, but they do not compile or probe freshness.
- Explicit build-script tracking or package inclusion of `.optic` can invalidate captures repeatedly.
- Publication has atomic visibility, without crash-durability, recovery, or concurrent-writer guarantees.
- Old prototype record formats are rejected. There are no data migrations or API compatibility promises.
- Windows, cross-target support, response-file compatibility, and special filesystem handling are outside the MVP.

Durable headers and cache pointers are limited to 1 MiB. Instance/evidence manifests are limited to
128 MiB. LLVM files are streamed, with a 1 MiB limit on an indexed definition or alias header.
These are resource budgets, not compiler limits or a bound on total process memory.

## Library and contributions

The `cargo-optic-api` package exposes the `optic` library. It provides the same capture policy,
capture outcome, search references, and typed evidence availability as the CLI.

Read [the architecture][architecture] for subsystem boundaries.
Before changing Rust code, read [the contributor instructions][contributors].
Start a code review with [the review guide][review].
The [planning index][planning] separates current contracts, completed work, and optional research.
The separate `prototype` branch is experimental context, not a feature-parity requirement.

[architecture]: https://github.com/connortsui20/optic/blob/main/docs/architecture.md
[contributors]: https://github.com/connortsui20/optic/blob/main/AGENTS.md
[review]: https://github.com/connortsui20/optic/blob/main/docs/review.md
[planning]: https://github.com/connortsui20/optic/blob/planning/docs/design/README.md
