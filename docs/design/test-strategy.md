# MVP test strategy

Tests establish the product claims in [PLAN.md](PLAN.md). The harness provides isolated fixtures and
useful diagnostics, not a language for describing scenarios.

Each invariant has one owning test layer. End-to-end journeys verify composition without repeating
every parser case through a real compiler.

## Test ownership

| Layer | Owns |
| --- | --- |
| Records unit tests. | Identifier parsing, constructors, versions, reference agreement, and round trips. |
| Store tests with real files. | Bounded durable input/output, atomic publication, pointers, and artifact reads. |
| Compiler unit tests. | Request/key normalization, private protocol, artifact classification, and LLVM scanner. |
| Compiler process tests. | Real Cargo freshness, driver execution, source spans, and optimized artifact stage. |
| API child-process tests. | Typed capture outcomes and the complete stored-evidence workflow. |
| CLI process tests. | Cargo subcommand discovery, arguments, references, stdout/stderr, and exit status. |
| Packaged installation test. | Archive completeness, installed CLI, and an external library consumer. |

Use semantic output assertions instead of whole-output snapshots. Byte equality is appropriate for a
specific source range or LLVM excerpt. Do not snapshot unstable compiler diagnostics or full IR.

Keep tests focused. When cases share a body, use a table-driven test. Follow `$rust-style` for
fixture placement, meaningful dimensions, module layout, and `#[track_caller]` assertions.

## Small shared helper

Create `cargo-optic-test-support` as an unpublished dev-only crate. Its consumers are existing
compiler, API, and CLI tests. A small concrete helper supplies:

- `TestWorkspace::new(fixture)` to copy one fixture into a private root.
- Accessors for the workspace, Cargo home, target, and temporary paths.
- `apply(&mut Command)` to set the child environment and working directory.
- Shared command failure diagnostics and assertions where multiple callers need them.

Keep command construction, binary discovery, and product expectations with their owning tests. Do
not build a command-builder facade, filesystem backend, public observer API, or scenario DSL.

Use path-only, versionless dev-dependencies. Packaging removes this unpublished dependency, as
specified in [installation](installation.md).

## Process isolation

Each fixture owns this layout:

```text
test-root/
|-- cargo-home/
|-- target/
|-- build/
|-- temp/
|-- observations/
+-- workspace/
    +-- .optic/store/
```

The store uses its ordinary workspace location. Do not add a product store-directory override only
for tests. Offline fixtures contain local dependencies and committed lockfiles, with no registry
downloads during the behavioral journey.

Resolve the installed Cargo/rustc toolchain before creating the isolated child. Clear the child's
environment. Supply the real toolchain binaries, necessary linker paths, and isolated fixture paths.
Include ordinary host variables required for process startup.

Preserve the existing rustup installation with an absolute `RUSTUP_HOME` and selected toolchain
path. Disable automatic toolchain installation. The fixture's configuration must not read the
developer's Cargo configuration or choose another toolchain. See
[rustup environment variables](https://rust-lang.github.io/rustup/environment-variables.html).

Apply test-specific variables only after the base environment. This makes compiler/wrapper
rejection, tracked environment inputs, and configuration changes explicit test inputs.

API tests need the same isolation even though they call Rust functions. Run their
environment-sensitive scenario in a fresh test-executable child, selected with an exact test name.
Set its environment through `Command` before startup. Never call `set_var` or `remove_var` in the
multithreaded parent. A mutex around such calls does not isolate other process threads. See
[Rust 2024 environment safety](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html).

Failures print the command, cwd, selected toolchain, status, stdout, stderr, and relevant
observation counts. Do not print arbitrary inherited environment values. CI timeouts bound stuck
compiler tests.

## Observing work

Cache correctness requires separate observations of actual driver compilation and selected-target
collection. The rustc driver invokes rustc in-process, so a rustc forwarding shim alone misses that
selected-target work.

Use one test-only Cargo forwarding shim in compiler integration tests. It invokes real Cargo and
interposes an observer around the installed wrapper. The observer records selected invocation mode
and forwards unchanged arguments and environment. Its path remains stable within the fixture.

Count selected collection separately from stopped probe interception. Count dependency compilation
separately so `--fresh` cannot pass by rebuilding the entire fixture.

Driver provisioning uses the resolved absolute rustc path. Test it with a compiler-unit-test-only
counter immediately beside that subprocess invocation. Exercise provisioning in separate
test-executable children sharing one fixture cache, then verify one cold build and no warm builds.
The counter is compiled out of product builds, with no public tracing setting or outcome type.

Every observation test starts with a positive cold count. Zero-only assertions can pass with broken
instrumentation. IDs, timestamps, executable mtimes, and directory contents supplement these counts
but do not replace them.

The CLI journey uses real `cargo optic` discovery without the observer shim. This verifies
installation and user behavior independently of the compiler integration instrumentation.

## Checkpoint A journey

Run successive requests in separate processes sharing one fixture:

| Step | Expected capture | Selected collections added | Driver builds added |
| --- | --- | --- | --- |
| Cold request A. | New ID and one history entry. | 1. | 1. |
| Unchanged A. | Same ID and completion time. | 0. | 0. |
| Edit a tracked source input. | New ID and new evidence. | 1. | 0. |
| Repeat edited A. | Same new ID. | 0. | 0. |
| Force `--fresh`. | Another new ID. | 1. | 0. |
| Repeat matching request. | Same forced-capture ID. | 0. | 0. |

Prove warm reuse creates no new analysis artifacts or completed-capture directory. Prove forced
analysis preserves eligible dependency artifacts. Do not assert elapsed milliseconds.

The driver and selected-target observations can live in separate focused tests that exercise the
same sequence. Do not add production plumbing merely to combine their counters into one test.

## Checkpoint A matrix

| Contract | Required cases |
| --- | --- |
| Requests. | Explicit package, supported target kinds, profile, features, invocation subdirectory, and missing target. |
| Profiles. | Warm reuse of a custom profile using stored Cargo artifact settings, not a name-to-settings map. |
| Existing core flow. | Empty history, ordinary/generic instances, list order, exact/substring search, zero results, and limits. |
| Cargo tracking. | Source edit, local dependency edit, build-script tracked file/environment, feature change, profile/config change, and RUSTFLAGS. |
| Cache identity. | Compiler/driver-source/recipe changes change keys, irrelevant attempt paths do not, and reordered equivalent features normalize. |
| Variant safety. | A/B/A under the same coarse key, including Cargo-tracked config or flags that Optic does not fingerprint. |
| Cargo observations. | Affirmative fresh artifact, stale receipt, unrelated failure, missing receipt, conflicting identity, and success without a matching artifact. |
| Compiler policy. | Default compiler, rejected compiler overrides, wrapper disabling and warning, and missing required components. |
| Store input. | Valid, malformed, incompatible, oversized, mismatched IDs, truncated files, and writer output over the same limit. |
| Publication. | Collection failure, staging write failure, pointer replacement failure, and final capture rename failure. |
| Output. | Captured/reused distinction, unchanged completion time, useful references, stderr warnings, and failed writes. |

For source edits, use deterministic file content changes and establish a distinguishable mtime when
the filesystem requires it. Do not add sleeps to hide a failing freshness test.

The critical failure sequence starts with a completed A, makes Cargo inputs stale, completes a new
compilation, then fails publication. A following request must never return A merely because the
failed attempt made Cargo fresh. Cover failures before and after pointer replacement.

Add a Git-backed root-package fixture with a build script that emits no `rerun-if-*` instructions.
Start without an Optic ignore rule. Verify that store initialization excludes its output before
compilation and that the next request reuses the capture. Also cover the non-Git default scan.
These cases must not rely on this repository's existing `/.optic` ignore rule.

Also test absent pointer, dangling pointer, and present corrupt capture separately. Corruption must
not become a miss. Verify old captures remain explicitly searchable, even when no longer candidates.

Use ordinary obstructed paths and malformed files for fault tests. If those cannot reach a specific
commit boundary deterministically, add a private test seam. Do not add general recovery, random
fault injection, or a virtual filesystem.

## Checkpoint B matrix

Repeat the entire cache journey after adding source and LLVM artifacts. The evidence recipe change
must prevent reuse of instance-only evidence.

Source tests cover whole functions/methods, generic sharing, unavailable external or generated
definitions, approved roots, Unicode, BOM/CRLF, and stored reads after checkout changes.

LLVM tests cover both supported optimization modes, expected CGU completeness, exact raw symbols,
quoted escapes, aliases, multiple placements, declarations, and absent standalone definitions. Known
unsupported incremental configuration must still capture/list/find/reuse successfully with its
stored warning.

Add table-driven classification cases for every unsupported mode listed in the show contract. Add
one ordinary cross-crate ThinLTO or fat-LTO process case that preserves capture/list/find/reuse and
supported source, with typed LLVM unavailability. Verify that unsupported classification skips LLVM
artifact requests and disassembly. Do not install another backend merely to test classification.

If the streaming scanner is subtle, keep a straightforward reference scanner in parser tests. Use
small hand-written LLVM samples for syntax cases and real compiler output for stage fidelity. Do not
rely on rustc producing every rare grammar shape in every release.

Store tests cover invalid references, missing/truncated artifacts, path escape and symlink
rejection, ranges past EOF, zero-length copying, and writer failures. A sparse artifact with a range
above 4 GiB proves offsets are not narrowed without allocating a huge buffer.

Test a very long unrelated LLVM line and a header beyond the documented parser limit. Check buffer
growth through the scanner's narrow test seam, not an operating-system-specific RSS threshold.

The CLI verifies that stdout contains only requested evidence. Unavailability produces no stdout, a
reason on stderr, and a failing status. Reads perform no compilation or current-source reread.

## Final gate

The [foundation](stabilization.md) owns formatting, Clippy, rustdoc, and Linux/macOS checks. The
[installation plan](installation.md) adds package verification and installed CLI/API journeys. All
run on the final integrated revision, followed by independent correctness and Rust-style review.

The final handoff maps each required contract to a test and records any unsupported behavior. A
green collection of unit tests without the real Cargo and installed journeys is insufficient.

Do not add coverage quotas, property-testing dependencies, continuous fuzzing, mutation testing,
benchmark infrastructure, or unsupported-platform fixtures in this MVP. A later concrete workload or
defect can justify them.
