# Narrow show

Narrow show reads captured source or optimized LLVM for one explicit instance reference. It starts
only after capture/list/find and both caches pass Checkpoint A.

It does not add query-driven capture, another output format, cross-capture matching, or comparison.

## Instance references

Add `InstanceRef` containing a full `CaptureId` and a `u64` instance ordinal. The ordinal identifies
the instance's immutable position in the stored manifest, before search sorting or limiting.

Its canonical text is `<full-reverse-hex-capture-id>:<decimal-ordinal>`. Do not add prefix lookup or
a second globally generated instance ID. Document that references remain meaningful only within that
exact capture and format.

Find returns each reference beside the existing instance record. Exact-match precedence, substring
semantics, ordering, and pre-limit totals stay unchanged. Two equal display names do not become one
instance. Identical symbols in different captures do not create cross-capture identity.

Parsing validates the representation. Resolving validates capture existence and ordinal bounds. An
invalid reference is an error, not unavailable evidence.

## Records and temporary ownership

Extend the existing instance manifest into the complete evidence manifest. Keep one source of truth
for each instance, artifact, and relationship. Do not introduce a second instance list.

The minimum stored vocabulary is:

| Record | Required meaning |
| --- | --- |
| Artifact entry. | Capture-local artifact ID, source/LLVM kind, generated file name, and byte length. |
| Byte range. | Half-open range with checked `u64` start and length. |
| Source relationship. | Artifact/range, display path, and starting line, or an explicit unavailable reason. |
| LLVM module entry. | Artifact, compiler module identity, and captured optimization stage. |
| LLVM definition entry. | Exact decoded raw symbol, module, kind, range, and direct alias target when present. |
| LLVM collection state. | Supported collected modules or `NotCaptured` with a specific configuration reason. |
| Provenance. | Exact compiler/LLVM identity, effective target, optimization, LTO, CGUs, and collection recipe. |

Keep artifact IDs distinct from instance ordinals because mixing them can select unrelated bytes. Do
not add separate identifiers for every table row without a harmful ambiguity.

The collected compiler result owns temporary output until publication finishes. The store copies
only declared artifacts, then validates references and lengths before the final rename.

When their layouts change, increment the durable evidence revision and driver protocol revision.
Include the evidence policy in cache identity. Repeat cache tests against this final schema.

## Captured source

Use the selected compiler's HIR and source map, not a second Rust parser.

For a supported local definition, obtain the whole item/body span. A definition-name or signature
span is insufficient. Classify synthetic instances and unsupported expansions before attributing
source. Do not substitute a macro call site for a generated definition.

Require both span endpoints to lie in the same loaded source file. Require ordered, in-bounds UTF-8
offsets. Restrict approved paths to the canonical selected package root.

Read the compiler's already-loaded source text and convert positions to file-relative offsets. Rustc
normalizes BOM and CRLF, so the stored snapshot contains the normalized UTF-8 bytes the compiler
used. Do not apply normalized offsets to a later raw filesystem read. See rustc's
[SourceFile contract](https://doc.rust-lang.org/beta/nightly-rustc/rustc_span/struct.SourceFile.html).

Snapshot each needed source file once per capture. Generic instances can share that snapshot and
range. The displayed path is metadata, not an instruction for the reader to reopen the checkout.

The initial unavailable cases include nonlocal definitions, unloaded source, unsupported expansion
or synthetic spans, and files outside the approved root. This includes source for external path
dependencies. Their instances and exact LLVM can still be legitimate selected-target evidence.

Unavailability is a normal typed result. Invalid persisted ranges or a missing declared snapshot are
store errors. A later checkout edit must not change the shown bytes.

## Optimized LLVM scope

Classify support using the actual selected compiler configuration, not a profile name.

The initial supported modes are the pinned LLVM backend with ordinary nonincremental local ThinLTO
or no LTO. Default release and explicit `lto = "off"` are required fixtures. Cargo's default
`lto = false` can still use local ThinLTO. See
[Cargo's LTO settings](https://doc.rust-lang.org/cargo/reference/profiles.html#lto).

Determine support before requesting extra artifacts. Initially unsupported modes include selected
incremental compilation, cross-crate ThinLTO, fat LTO, linker-plugin LTO, and another backend.

For a known unsupported mode, capture still succeeds with instances, supported source, and a durable
`NotCaptured(UnsupportedConfiguration)` reason. Warn during capture and reuse. Do not disable
incremental compilation or change optimization, LTO, CGUs, symbol mangling, emits, or linking to
obtain LLVM output.

This protects ordinary development capture/list/find. It does not silently claim optimized LLVM
support for all previously accepted profiles.

## Selecting the artifact stage

Use the exact-version driver's configuration callback to retain temporary codegen output in its
private attempt directory. Preserve the selected build's compilation configuration.

Retain every expected regular CGU module, including modules without recorded function placements.
Derive expected paths from rustc's output naming and CGU information. Do not glob for the first
plausible bitcode file.

For `Collected` evidence, durable validation requires a module for every CGU named by an instance
placement. Extra modules without placements remain valid. This checks collection completeness, not
body location: exact symbol queries still search all modules after optimization. `NotCaptured`
evidence does not require LLVM modules.

The verified mapping on Rust 1.98.1 is:

| Effective mode | Expected optimized module |
| --- | --- |
| No LTO. | Each regular CGU's final `.bc` output after its configured LLVM pipeline. |
| Local ThinLTO. | Each regular CGU's `.thin-lto-after-pm.bc` output. |

Existing [research fixtures](../research/fixtures/README.md) are the starting evidence, not proof
that this mapping works in the product. Promote a focused reproduction into compiler tests. Inspect
the matching compiler source and record a stable source/reproducer link beside the implementation.
Resolve this artifact-stage check before exposing LLVM as available.

The promoted `tests/fixtures/stage-proof.rs` and compiler integration test passed on the pinned commit
`48a229ceaefd4985c50990b14116b6d856af0985`, using LLVM 22.1.8. Both modes produced four regular
modules. Paired invocations preserved effective optimization, LTO, CGU count, and output types.
Both linked programs ran successfully. The retained modules contained constant folding that their
`no-opt.bc` predecessors did not contain. The test runs in a cleared child environment.

The matching compiler source writes [no-LTO bitcode after optimization](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_codegen_ssa/src/back/write.rs#L819-L840)
and [local ThinLTO bitcode after its pass manager](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/compiler/rustc_codegen_llvm/src/back/lto.rs#L778-L782).
Worker proof commits are `d98f141` and its process-isolation update in `76a0308`. Integrated product
acceptance and both-host CI remain separate requirements.

Disassemble each selected module once with the matching sysroot's `llvm-dis`. Store that unchanged
text as the durable LLVM artifact. Intermediate bitcode and earlier stages remain temporary.

A missing expected artifact, disassembler failure, or malformed supported output is a hard capture
error. Do not downgrade it to unsupported configuration or select an earlier stage.

Do not use `--emit=llvm-ir` as an unverified shortcut. Earlier experiments found that it can alter
local ThinLTO behavior. The acceptance test must prove the chosen collection method retains the
effective optimization configuration and expected stage.

## Exact indexing

Use LLVM's symbol syntax, including quoted names and escape decoding. Match the compiler's raw
symbol exactly. Display names and demangling are not evidence joins. The
[LLVM language reference](https://llvm.org/docs/LangRef.html#identifiers) owns this syntax.

Scan each stored text module once and record definition or alias ranges. Keep offsets as `u64`. Use
a small bounded scanner for the required top-level constructs, not an LLVM object model or another
native-library dependency.

Read fixed chunks so an enormous unrelated line cannot cause an unbounded allocation. Bound the
accumulated definition/alias header to 1 MiB. This is an explicit parser resource budget, not an
LLVM language limit. Stream past unrelated globals and body content while tracking ranges.

Recognize ordinary definition headers and direct aliases with their modifiers. Resolve a direct
alias chain only through exact names in the same module. Detect cycles. Expression aliases and
ifuncs can remain explicitly unsupported. Never guess a target from a substring.

Return every exact body across captured modules, in deterministic module/range order. Preserve alias
provenance when an exact direct alias resolves to a body. Do not select only the first CGU
placement, since later optimization can import or move a body.

If complete supported modules have no exact standalone definition, report that fact. Do not call it
"optimized away": renaming or merging can also remove the exact standalone symbol.

A returned function excerpt is not promised to be an independently assemblable LLVM module. Types,
attributes, and metadata can remain elsewhere in the captured module.

## API and CLI

The evidence API returns source or LLVM availability plus capture-scoped artifact ranges. The store
owns path resolution and filesystem validation. The caller supplies a writer to `copy_evidence`. The
API does not return unrestricted filesystem paths.

Copy a finite validated range through a fixed buffer. Reject premature EOF and invalid bounds.
Propagate writer errors and document that partial output is possible. Do not add a public
maximum-output-size option or load the full module to display one body.

Support only these commands:

```console
cargo optic show --instance INSTANCE_REF --output source
cargo optic show --instance INSTANCE_REF --output llvm
```

Successful stdout contains evidence text without color or surrounding prose. Put origin/stage and
multi-body diagnostics on stderr. Separate multiple LLVM excerpts deterministically with a newline.
Unavailable evidence produces an explanatory stderr message, empty stdout, and a nonzero exit.
Malformed data remains an error with its storage context.

Store reads do not compile or capture. `Optic::open` can still perform the existing workspace
metadata discovery. Source and LLVM bytes come only from the completed capture.

## Acceptance

Repeat all Checkpoint A cache sequences with the final evidence policy and artifacts. In addition:

- Find references still select the correct records after sorting and limiting.
- Source includes whole ordinary functions and methods, with shared generic source.
- CRLF, BOM, Unicode before a span, and paths containing spaces preserve the byte contract.
- Unsupported source provenance returns a reason instead of nearby text.
- Stored show remains unchanged after the source checkout changes.
- Default-release local ThinLTO and explicit no-LTO provide the expected optimized stage.
- Multiple CGUs, quoted symbols, direct aliases, declarations, and absent bodies have exact results.
- Unsupported incremental LLVM still permits capture/list/find/reuse with a warning.
- Missing required supported artifacts prevent publication.
- Large unrelated LLVM content does not need memory proportional to the whole module.
- Truncated artifacts, out-of-range offsets, unknown IDs, and corrupt manifests return errors.

The [test strategy](test-strategy.md) assigns these checks to their narrowest useful boundary.
