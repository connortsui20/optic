# Using Cargo Optic

The [README](../README.md) covers installation and a first capture. Run each command from the Cargo
workspace to inspect. For capture, use the compiler and components in
[rust-toolchain.toml](../rust-toolchain.toml). Commands that read stored evidence do not require the
compiler used for capture.

## Select a target

Rustup selects the toolchain automatically in the Optic checkout. For another workspace, select that
toolchain with `cargo +TOOLCHAIN optic capture`. Replace `TOOLCHAIN` with the toolchain file's
`channel` value.

Each capture requires one package name, one target, and one profile:

```console
cargo optic capture -p my-crate --lib --release
```

Replace `my-crate` with the package name from its `Cargo.toml`. Select the target with one of these
options:

| Option | Target |
| --- | --- |
| `--lib` | The package's library target. |
| `--bin NAME` | A binary with the given Cargo target name. |
| `--example NAME` | An example with the given Cargo target name. |
| `--bench NAME` | A benchmark with the given Cargo target name. |

Use `--release` or `--profile NAME` to select the profile. `--profile dev` is valid, but incremental
compilation prevents LLVM output. Cargo feature options are `--features a,b`, `--all-features`, and
`--no-default-features`.

## Find an instance

List completed captures, then search one capture:

```console
cargo optic list-captures
cargo optic find --capture CAPTURE_REF kernel --limit 20
```

Replace `CAPTURE_REF` with a capture ID from the output. Captures appear in descending completion
time, with capture IDs in ascending order for ties.

Search prefers exact definition names, concrete instance names, and raw compiler symbols. Otherwise,
it uses a case-sensitive literal substring. Results have a deterministic order and a default limit
of 20. The selected compilation can include dependency instances, but capture does not analyze every
dependency target separately.

## Read evidence

Replace `INSTANCE_REF` with the value from a search result's `Reference` line:

```console
cargo optic show --instance INSTANCE_REF --output source
cargo optic show --instance INSTANCE_REF --output llvm
```

An instance reference identifies one stored instance in one immutable capture. It does not identify
the same function across captures. Invalid references are errors.

Source output comes from the compiler's captured text. Later edits to the checkout do not change it.
LLVM output contains one or more matching function bodies from the captured optimization stage.
These excerpts use exact compiler symbols. They do not form a complete, independently assemblable
LLVM module.

Evidence goes to stdout, and diagnostics go to stderr. If evidence is unavailable, `show` explains
why and exits with a nonzero status. It does not start another capture. Listing, search, and
evidence reads can run Cargo metadata, but they do not compile or check freshness.

## Reuse a capture

Repeat the same capture command to let Cargo check freshness. A fresh result prints `Reused` and
preserves the original capture ID, completion time, and evidence. Reuse does not compile the
selected target or driver again, copy evidence, or add another capture.

Use `--fresh` to force analysis of the selected target:

```console
cargo optic capture -p my-crate --lib --release --fresh
```

Forced analysis retains compatible driver and dependency caches. Changes to Cargo-tracked inputs
also produce a new capture. Cargo metadata locates packages and targets. Cargo's build freshness
decision determines whether Optic can reuse evidence.

## Storage

Completed captures live in `.optic/store` under the inspected workspace. Drivers live in
`$CARGO_HOME/optic/drivers`, normally `~/.cargo/optic/drivers`.

Before compilation, Optic creates `.optic/.gitignore` containing `*` followed by a newline. This
keeps Optic's own files out of Cargo's default package scans. Different existing contents cause an
error and remain unchanged. Explicit package inclusion or build-script tracking of `.optic` can
still invalidate captures repeatedly.

Each new collection adds a capture and can leave another selected-target build artifact. Interrupted
staging directories and obsolete drivers can also remain. There is no automatic retention or
cleanup.

Captures require one writer per workspace and serialized driver provisioning within each Cargo home.
Publication makes a completed capture visible atomically. It does not provide crash durability,
recovery, or concurrent-writer guarantees.

## Supported configurations and evidence limits

### Compiler and platform

The toolchain file defines the supported compiler for capture on Linux and macOS. Optic discovers
the active compiler and builds a matching driver with its `rustc-dev` and `llvm-tools` components.
The driver uses unstable rustc APIs. Other compiler versions are untested, and changes to those APIs
can prevent driver compilation.

If the driver builds with another compiler version, capture can retain instances and supported
source evidence. LLVM capture requires the compiler revision tested by this release of Optic.

Compiler overrides through `RUSTC`, `CARGO_BUILD_RUSTC`, or Cargo's `build.rustc` are unsupported.
Configured compiler wrappers are disabled with a warning. Wrapper composition is unsupported.

Windows, cross-compilation, response files, and special filesystem configurations such as noexec
mounts are unsupported. Old record formats are rejected. There are no data migrations or API
compatibility guarantees.

### Source

Source capture supports local definitions with exact compiler-loaded spans within the selected
package root. External definitions, unsupported macro expansions, and synthetic spans can have no
available source. Snapshots contain rustc-normalized UTF-8, including BOM removal and CRLF
normalization. Generic instances can share the same source range.

### LLVM

Optimized LLVM output supports nonincremental builds with no LTO or local ThinLTO. Local ThinLTO
operates within one crate. Cargo's `lto = "thin"` selects cross-crate ThinLTO instead.

These configurations do not provide LLVM evidence:

- Incremental compilation.
- Cross-crate ThinLTO (`lto = "thin"`).
- Fat LTO (`lto = true` or `lto = "fat"`).
- Linker-plugin LTO.
- A compiler backend other than LLVM.

Optic preserves the selected build configuration. Unsupported LLVM configurations retain capture,
search, and supported source evidence, with an explicit warning about unavailable LLVM output.
Missing required artifacts in a supported configuration are errors.

A missing exact LLVM symbol does not prove that optimization eliminated the function. Multiple exact
bodies remain separate excerpts.

### Resource limits

| Data | Limit |
| --- | --- |
| Durable headers and cache pointers. | 1 MiB each. |
| Instance and evidence manifests. | 128 MiB each. |
| An indexed LLVM definition or alias header. | 1 MiB. |

LLVM files are streamed. These limits bound individual records and headers, not total process memory
or the compiler's inputs.
