# MVP review follow-up

The user requested a new PR for two reproduced regressions in `main` at `bae4ca0`, plus related cleanup.
The completed MVP remains the baseline. This follow-up does not add features or compatibility layers.

The follow-up is complete in [PR #18](https://github.com/connortsui20/optic/pull/18), merged as
`c94cc22` after explicit user approval. The merged tree matches the reviewed and tested `f84a25b`.

This record preserves completed work, not active worker assignments. The task branches and worktrees
are obsolete. Temporary evidence paths identify past runs whose local files are no longer available.
Recorded results and committed regressions remain available without those files.

## Fixes and ownership

- The integration owner fixes Cargo executable discovery and adds isolated process regressions.
- A compiler worker replaces repeated source-prefix scans with rustc's existing line index.
- A separate reviewer audits nearby capture, evidence, and process boundaries for concrete defects.
- The integration owner records any additional defect before implementing its correction.

Preserve explicit `CARGO` selection and the selected executable's invocation path. PATH lookup must
skip non-executable entries and retain ordinary symlink and relative-directory behavior. Do not add
Windows support, shell invocation, compiler overrides, or wrapper-composition machinery.

Use `rustix::fs::accessat` with effective-user execute access for PATH candidates. Rustix is already
in the resolved dependency graph through tempfile. A direct dependency avoids unsafe local FFI or
manual permission-bit emulation. Keep the existing path-selection loop and explicit-path behavior.

Source line lookup must use the pinned compiler's source map and preserve one-based normalized line
numbers. Test first and later lines, multiple files, and repeated concrete instances. Reproduce the
large-file cost without a machine-dependent timing assertion or another source-line cache.

## Verification and delivery

- [x] Reproduce each reported regression and add focused coverage.
- [x] Complete both fixes under the full Rust style rules.
- [x] Audit related code and resolve any additional reproducible issues found.
- [x] Run workspace tests, standalone formatting, Clippy, and public/private rustdoc.
- [x] Run archive installation, the external API consumer, and self-hosting verification.
- [x] Complete independent correctness and Rust-style review of the final diff.
- [x] Open the corrective PR and verify Linux/macOS CI on its final revision.
- [x] Record results and the PR link on `planning`.
- [x] Merge the approved revision and sync main and planning.

Use the existing isolated fixture framework. Never change the parent process environment, uninstall
toolchain components, or modify user Cargo configuration to reproduce errors. Registry DNS failure
is a verification blocker, not a reason to change product code or disable archive verification.

The user reviewed PR #18 and explicitly requested its merge. The exact-revision CI gate passed,
the PR is merged, and planning is rebased onto main. No registry publication is authorized.

## Execution evidence

The integration used `ct/fix-mvp-review` from `bae4ca0`. The source worker used a separate
`ct/fix-source-line-index` worktree. Both followed the complete Rust style rules.

The Cargo process regression fails before the permission fix: `cargo -V` succeeds while direct
`cargo-optic optic list-captures` fails with `Permission denied`. Three new process tests pass after
the fix. They cover unset, empty, and bare-name `CARGO`, explicit-path failures, PATH precedence,
relative PATH directories, and preservation of the executable symlink's invocation name.

The independent audit covered source identity/ranges, LLVM indexing and aliases, Cargo selection and
freshness, and publication failures at `bae4ca0`. All 170 relevant tests passed. An additional
isolated workspace verified member features, warm reuse, shared-file source attribution, and LLVM
lookup. No additional reproducible defect was found in those inspected paths and exercised cases.
The audit used `/tmp/optic-followup-repro.VC9jd4C9`, which is no longer available.
The audit result does not establish correctness for all possible inputs.

## Corrective PR

[PR #18](https://github.com/connortsui20/optic/pull/18) contains both corrections at `f84a25b`.
The Cargo change is `851f408`. The source-line change is `f84a25b`, from worker commit `6d5fcee`.
The source test covers first and later lines, multiple files, repeated monomorphizations, Unicode,
and BOM/CRLF normalization. It passes before and after the algorithm change and preserves semantics.

All 241 workspace tests, standalone formatting, Clippy, and public/private rustdoc pass locally.
Independent correctness and Rust-style reviews approve the complete diff. Installed verification
and both-host CI also pass.

## Source-line cost reproduction

The reproduction isolates the standalone driver process, not total Cargo capture time. It uses
4,000 generated public functions in one 802,890-byte file. Each function has one comment with four
repeated padding phrases and returns `value.wrapping_add(index)`. Source is identical in both variants.

Driver binaries use the production driver build flags from `driver.rs`. Both invoke the pinned
Rust 1.98.1 compiler with `--crate-type rlib --edition=2024 -C opt-level=0 -C codegen-units=1
-C lto=off -C debuginfo=0`. After one warmup per variant, five alternating before/after pairs run
with the same source and output locations. The baseline uses `bae4ca0`; the candidate uses `6d5fcee`.

| Wall time in seconds | Before | After |
| --- | --- | --- |
| Pair 1. | 13.647 | 0.298 |
| Pair 2. | 13.599 | 0.294 |
| Pair 3. | 13.592 | 0.293 |
| Pair 4. | 13.604 | 0.291 |
| Pair 5. | 13.620 | 0.286 |
| Median. | 13.604 | 0.293 |

The median time ratio is 46.4 on this workload. Manifests and source snapshots match byte-for-byte.
The run used `/tmp/optic-source-line-fix/target/source-line-bench`.
The runner, generated input, binaries, output, and raw timing files are no longer available.
The methodology and timing table remain here as historical evidence. CI has no timing threshold.

The timed process includes compiler startup, parsing, analysis, Optic collection, code generation,
and artifact output. It excludes driver compilation, parent-side manifest processing, LLVM
disassembly, and the comparison/archive steps.

## Final review and merge decision

Independent correctness and style reviews approve `f84a25b`. The complete installed workflow also
passed at that revision. The run used the temporary directory `/tmp/optic-install.Y4Q6DcbE`.

An extra standalone-driver rustdoc invocation finds three existing `private_intra_doc_links`
warnings in the crate documentation. The links describe private entry points in a binary that has
no public library API. This extra lint invocation is outside the passing workspace/CI documentation
gate. The user then requested merging the reviewed PR as-is. No additional lint allowance or
whitespace change was made after that approval. These nonblocking observations are not feature work
or additional requirements for this correction.

## Completed verification

[CI run 34001278916](https://github.com/connortsui20/optic/actions/runs/34001278916) passes all jobs on
`f84a25b469823d024c350035f93c740c8d03faac`. This includes Linux/macOS workspace and installation
tests, plus formatting, Clippy, and workspace rustdoc. GitGuardian also passes.

PR #18 merged on 2026-09-06 at 00:30:42 UTC as `c94cc22887dff08ac3df7a2b7eb7a2e8bf7d73f3`.
The shared checkout is clean on main. Planning remains confined to design documents and research
on top of main. No user store, worker worktree, or preexisting stash was removed.
