# Capture and driver reuse

Cargo owns build freshness. Optic owns the association between one analysis invocation and one
immutable completed capture. Neither a request hash nor a missing driver manifest proves freshness.

This contract completes Checkpoint A before source or LLVM implementation begins.

## Two caches

The driver cache reuses an executable compatible with the exact compiler and driver recipe. The
capture cache reuses evidence only after Cargo positively verifies that the selected target remains
fresh.

Ordinary Cargo dependency artifacts remain in Cargo's configured target/build directories. Optic
does not copy or replace Cargo's dependency cache.

| Request state | Cargo build/probe calls | Selected-target work | Result |
| --- | --- | --- | --- |
| No candidate. | One collection. | Compile with a new analysis token. | New capture. |
| Candidate is fresh. | One probe. | No selected compilation. | Existing capture. |
| Candidate is stale. | One stopped probe, then one collection. | Compile with a new token. | New capture. |
| `--fresh`. | One collection, without a probe. | Compile with a new token. | New capture. |

Metadata and compiler identity discovery are separate from these build/probe counts.

## Driver provisioning

Resolve the actual default rustc executable and sysroot once for the operation. Use that same
executable for driver compilation and record it in compiler identity.

Place reusable drivers under `$CARGO_HOME/optic/drivers/<driver-key>/`. Resolve the ordinary default
Cargo home when the environment does not set it. The executable path stays stable across captures
and processes. Do not build executable drivers under the general temporary directory.

The key covers exact compiler identity and sysroot, host, every embedded driver source, private
protocol revision, and all fixed driver build options. When those inputs change meaning, increment
the recipe revision. Use a maintained digest implementation, not a hand-written hash or
`DefaultHasher` as a durable format.

Build a missing driver in a sibling temporary directory. Validate the completed entry and rename it
into the final location. Build with the exact resolved compiler and scoped bootstrap required for
rustc-private driver code. Do not pass that bootstrap permission to the selected user crate.

An absent entry triggers compilation. A present malformed or incompatible entry returns an error. Do
not guess another cache location, repair permissions, or search for an approximately compatible
compiler. Missing rustc-dev or LLVM components get actionable diagnostics.

The single-writer precondition in [architecture](mvp-architecture.md#publication-and-storage) covers
shared driver provisioning. No lock implementation is required.

## Request key and candidate

Keep one candidate pointer per normalized explicit request key. The pointer contains the key,
analysis token, capture ID, driver key, and evidence-policy revision.

The request key includes resolved workspace and invocation directories, package/target selection,
profile, and explicit features. It also includes Cargo executable/version, compiler identity,
effective target/build locations, driver key, and evidence-policy revision. Canonicalize unordered
feature selection. Do not include the attempt directory, analysis token, or completion timestamp.

This is a conservative candidate partition, not a reconstruction of Cargo's fingerprints. Cargo
remains responsible for source, dependencies, build scripts, configuration, flags, and tracked
environment changes. Do not hash the entire source tree or environment.

The pointer belongs to the store. The compiler returns the prepared request identity needed to
locate it. The capture layer decides whether to probe or collect.

During successful collection, also retain the selected Cargo artifact's observable identity in
the completed capture. It includes package ID, target name/kinds/source path, enabled features,
output filenames/executable, and Cargo's reported profile fields. Those fields are optimization
level, debug info, debug assertions, overflow checks, and test mode. Normalize unordered collections.

Fresh probing compares those observed fields with the stored observation, excluding the `fresh`
flag itself. The requested profile name remains a command input and part of the request key.
Do not translate names such as `release` or a custom profile into guessed effective settings.

## Analysis token invariant

Every actual selected-target compilation uses a newly generated token. A token that already names
published evidence can only be probed, never compiled again.

Pass the token through the selected-target marker already used by `cargo rustc`. Cargo sees it as
part of that target's arguments. The driver recognizes and removes the marker before running the
user compiler, so it changes the analysis identity without changing Rust semantics.

Keep the wrapper executable stable. Attempt-local receipt paths travel through the private driver
environment, not through changing selected-target arguments.

This invariant protects configuration transitions such as A to B to A. It also protects failed
publication followed by another request. Even if Cargo retains another build variant, its token
cannot acquire evidence from a different compilation.

Do not achieve `--fresh` through `cargo clean`, a new target directory, or changed dependency flags.
Only the selected target needs a new analysis token.

## Probe and collection

Use distinct prepared probe and collection paths, not a boolean spread through unrelated helpers.

A probe runs Cargo with the candidate token and the driver's probe mode. It consumes Cargo's
structured messages, using the existing Cargo metadata support where suitable.

If Cargo invokes the selected wrapper, the wrapper verifies the selected invocation, writes an
attempt-local stale receipt, and stops before selected-target analysis. Nonselected invocations
continue through the ordinary compiler forwarding path.

A successful probe permits reuse only when all these conditions hold:

- Cargo exits successfully.
- Exactly one matching selected-target artifact reports `fresh: true`.
- The selected artifact agrees with the prepared target and stored Cargo artifact observation.
- No selected-compilation or stale receipt contradicts that result.
- The candidate points to complete, valid evidence of the required revision.

Success without affirmative freshness, conflicting messages, or an unexpected selected invocation is
an error. Do not treat absent evidence as an empty collection.

A failed probe with a verified stale receipt permits exactly one collection attempt with a new
token. This receipt proves interception, not successful compilation or the absence of another Cargo
failure. The subsequent complete Cargo invocation must independently succeed.

A failed probe without that receipt returns an error. Cancellation does not authorize a retry. There
is no general retry loop. Actual collection requires successful Cargo completion and the matching
selected-driver result. A failure publishes nothing.

Redirect probe stderr into one attempt-local file while parsing stdout. Retain structured error
diagnostics too, and relay warnings separately. On successful freshness, replay diagnostics. On an
unclassified failure, replay the complete file. On stale retry success, show the collection's
diagnostics without the intentional probe-abort message. On retry failure, replay both attempts'
diagnostics. Do not parse Cargo's free-form stderr to classify errors.

This buffers probe progress until it finishes. Accept this limitation in the MVP. A stale probe can
also repeat a concurrently failing dependency once, but it can never turn a failed collection into a
successful result.

## Publication ordering

The old candidate can remain during probing and collection because its token is never compiled
again. Warm reuse performs no durable writes.

Publish new evidence in this order:

1. Collect with a new token and retain ownership of its temporary artifacts.
2. Prepare the complete staging directory, including validated bounded records and artifacts.
3. Atomically replace the request pointer with one naming the future capture ID and new token.
4. Rename the staging directory into the completed-capture namespace.
5. Return `Captured` without a required fallible cache write.

The store's publication operation owns steps 2 through 4. Pointer replacement occurs immediately
before the existing final capture rename.

A failure before pointer replacement preserves the old candidate. A failure after replacement but
before capture commit leaves a dangling pointer, which is a miss. Never reuse the token from a
dangling pointer. Never restore eligibility by scanning historical capture headers.

If the final rename succeeds, the completed capture and pointer agree. Printing the result can still
fail, but that does not roll back or invalidate publication.

## Read outcomes

| Observed state | Behavior |
| --- | --- |
| Pointer absent. | Collect with a new token. |
| Referenced capture directory absent. | Treat the pointer as a miss and use a new token. |
| Present pointer malformed, oversized, or incompatible. | Return a store error. |
| Present capture incomplete, corrupt, or incompatible. | Return a store error. |
| Complete capture and affirmative Cargo freshness. | Return the original record without rewriting it. |
| Complete capture and verified stale interception. | Collect once with a new token. |
| Unclassified Cargo failure. | Return an error without publication. |

Candidate validation uses bounded metadata reads and artifact existence/length checks. It does not
reread current source or hash every stored LLVM byte.

Changing the evidence policy selects another key. An unsupported old format remains an error when
explicitly queried, without migration code. A complete current-format `NotCaptured` result is valid
for reuse, with its stored warning.

## Public result and efficiency

The [architecture](mvp-architecture.md#public-workflow) defines typed capture policy and outcome.
The CLI adds `--fresh` and prints `Captured` or `Reused` with the same reference format.

A reuse returns the original ID and completion time. It adds no history entry, evidence copy, driver
build, selected-target compilation, or analysis artifact. Small temporary probe receipts and normal
Cargo discovery are allowed.

Each real collection can leave another selected-target artifact and adds one durable capture. The
MVP has no retention policy. This is different from rebuilding on every unchanged request.

## Required proof

The [test strategy](test-strategy.md) owns the full matrix. Before Checkpoint B, prove cross-process
warm reuse, independent driver reuse, tracked invalidation, A/B/A transitions, dependency reuse,
`--fresh`, and failures on both sides of pointer replacement.

Tests must observe actual driver builds and distinguish selected probe interception from real
collection. Stable IDs, unchanged mtimes, and elapsed time alone do not prove that work was avoided.
