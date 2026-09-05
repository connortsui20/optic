# Contract-test matrix

This matrix contains the cases that can change correctness, fidelity, or a product claim. It is an
implementation reference, not required product reading.

This matrix is a historical snapshot. Its status values do not report current test coverage. Read
the [`design plan`](../../design/PLAN.md) for the implemented boundary.

`Observed` means that a local experiment reproduced the case. `Tested` means that an automated test
covers the required behavior. `Implemented` means that the behavior has no contract test. `Pending`
means that the implementation still needs a contract test.

## Client and agent behavior

| Case | Status | Required behavior |
| --- | --- | --- |
| Stateless JSON query | Tested | Use explicit opaque IDs and return a versioned result. |
| Progress events | Pending | Return versioned JSON Lines events with one terminal result. |
| Ambiguous lookup | Tested | Return every candidate and a stable error code. |
| Independent clients | Pending | Do not use one shared current selection. |
| Client context | Pending | Isolate state by context ID. |
| Context revision conflict | Pending | Reject the older update without overwriting newer state. |
| Idempotent retry | Pending | Return the original operation for a repeated idempotency key. |
| Concurrent readers | Pending | Query complete collections during an import. |
| Read-only query | Pending | Do not update a context or access time without an explicit request. |

## Cargo and process behavior

| Case | Status | Required behavior |
| --- | --- | --- |
| No existing compiler wrapper | Observed | Install the outer capture wrapper and preserve rustc arguments. |
| Existing global wrapper | Tested | Preserve its arguments, streams, exit status, and position. |
| Warm dependency graph | Tested | Compile the selected target once and reuse dependency artifacts. |
| Existing workspace wrapper | Tested | Preserve its position so Cargo artifact hashes do not change. |
| Global and workspace wrappers | Tested | Reconstruct the original chain once without recursion. |
| Compiler-cache wrapper | Pending | Validate the requested evidence files. If rustc execution is unknown, report it. |
| Compiler probes | Tested | Pass through without evidence flags. |
| Fresh Cargo unit | Observed | Reuse prior evidence or report unavailable evidence. |
| Missing evidence for a fresh unit | Pending | Require an explicit rebuild after an unexpected miss. |
| Build script and procedural macro | Observed | Treat each compiler invocation as a host unit. |
| Cross compilation | Observed on wasm | Determine host or target for each rustc invocation. |
| Nested Cargo | Pending | Detect the inherited collection and prevent wrapper recursion. |
| Concurrent Cargo processes | Pending | Isolate invocation directories and stream ownership. |
| Shared Cargo target cache | Pending | Operate one collection at a time under a file lock. |
| Separate Cargo cache partitions | Pending | Permit concurrent collections. |
| Cancellation | Pending | Relay signals and publish no partial compiler evidence. |
| Configured build directory | Observed | Trust Cargo events and rustc arguments instead of directory layout. |
| Renamed or repeated dependency | Pending | Resolve each `--extern` path to its physical producer. |

## Cargo target modes

| Case | Status | Required behavior |
| --- | --- | --- |
| Library and binary | Observed | Keep separate compiler records. |
| Unit and integration tests | Observed | Store Cargo mode and actual rustc flags. |
| Benchmark without a harness | Observed | Do not infer harness behavior from Cargo mode. |
| Example | Observed | Store target kind, crate types, and outputs. |
| Metadata-only `cargo check` | Observed | Report `no-codegen-requested` and omit LLVM evidence. |
| Forced code generation during check | Observed | Label the collection as a modified build. |
| Doctest | Observed | Use the rustdoc builder wrapper and capture response files and input. |
| Compile-only doctest | Observed | Use a recorded no-op test tool. |
| Existing doctest wrapper | Pending | Preserve wrapper order. |
| `compile_fail` doctest | Observed | Keep diagnostics but publish no reusable body evidence. |
| Ignored or `should_panic` doctest | Pending | Preserve test semantics and execution policy. |

## Compiler pipeline

| Case | Status | Required behavior |
| --- | --- | --- |
| LTO disabled | Observed | Keep optimized CGU modules and expect no ThinLTO stages. |
| Local ThinLTO | Observed | Index every saved stage. |
| Cross-crate ThinLTO | Observed | Accept dependency and sysroot partitions. |
| Fat LTO | Observed | Scan the merged module with bounded memory. |
| Incompatible fat-LTO arguments | Observed | Diagnose the conflict and preserve the requested configuration. |
| One or many requested CGUs | Observed | Record the observed modules instead of the requested count. |
| Repeated output flags | Observed | Parse the complete output set and preserve explicit paths. |
| User-selected temporary paths | Observed | Preserve the path or omit the affected capture channel. |
| Direct `--emit=llvm-ir` | Observed | Record a separate capture method. |
| Incremental reuse | Observed | Trust invocation-owned directories and ignore stale glob results. |
| Non-LLVM backend | Pending | Preserve non-LLVM evidence and report the LLVM adapter as unavailable. |
| Linker-plugin LTO | Pending | Detect bitcode-bearing objects without claiming normal LTO stages. |
| Profile-guided optimization, coverage, or sanitizer | Pending | Record the configuration and compiler-generated support code. |
| Custom target specification | Pending | Snapshot and hash the exact target specification. |
| Split debug artifacts | Pending | Index companion files without assuming one file per object. |
| Missing `rustc-dev` | Implemented | Show the exact `rustup component add` command and stop. |
| Invalid identity protocol | Tested | Reject wrong versions, rustc commits, lengths, counts, and trailing bytes. |
| Cached rustc driver | Observed | Reuse a compatible helper for the same host, rustc commit, and source digest. |

## Identity

| Case | Status | Required behavior |
| --- | --- | --- |
| Type and constant instances | Observed | List all concrete instances under one definition. |
| Public path and canonical symbol path | Tested | Join them through rustc's exact raw symbol. |
| Vortex Bloom insertion | Observed | Find the optimized body and its vectorized eight-lane update. |
| Unused generic argument | Observed | Permit several instances to share one body. |
| LLVM alias | Observed | Connect every valid instance to the shared LLVM body. |
| Duplicate short name | Observed | Require a qualified path or return ambiguity. |
| Trait and inherent methods | Observed | Keep their definition identities separate. |
| Closure and async function | Observed | Keep generated and inline facts separate from the source function. |
| Dynamic dispatch and virtual-method table | Observed | Index anonymous globals without inventing a Rust identity. |
| One item in several CGUs | Observed | Store every placement and linkage. |
| Exported symbol | Observed | Connect the Rust and external names through evidence. |
| Mixed v0 and legacy mangling | Observed | Detect mangling for each symbol. |
| Missing debug information | Observed | Report weaker legacy and exported-symbol relationships. |
| Optimized clone or fragment | Tested | Report no exact body until LLVM lineage evidence connects it. |
| Linker removal or code folding | Pending | Record whether each object symbol remains after linking. |
| Cross-build symbol changes | Observed | Use structural matching and report confidence. |

## Source and security

| Case | Status | Required behavior |
| --- | --- | --- |
| Workspace source | Observed | Capture a baseline and validate it after compilation. |
| Path with spaces | Observed | Parse Makefile escaping. |
| Generated `OUT_DIR` source | Observed | Add it after compilation and mark it as generated. |
| Dependency generic source | Observed | If downstream dep-info omits it, capture the package source. |
| Debug checksum | Observed | If a checksum is available, validate the stored source bytes. |
| Environment dependency | Observed | Store the name and a redacted or hashed value. |
| Included source or binary | Pending | Treat the content as a sensitive compiler input. |
| Source edit during compilation | Pending | Compare the baseline, compiler evidence, and final read. |
| Symbolic link or remapped path | Pending | Keep recorded and resolved paths separate. |
| Non-UTF-8 path | Pending | Store native bytes and a separate display string. |
| Windows drive or UNC path | Pending | Parse drive syntax without confusing the dep-info target separator. |
| Arbitrary debug path | Pending | Reject reads outside configured roots without permission. |
| Undeclared build input | Known limit | Report incomplete provenance. |
| Hostile build code | Known limit | Require explicit collection and an external sandbox. |

## Parsing and storage

| Case | Status | Required behavior |
| --- | --- | --- |
| Matching LLVM tools | Observed | Locate tools through the actual compiler sysroot. |
| Mismatched LLVM tool | Observed | Keep unreadable bitcode and report the version problem. |
| Quoted symbols and nested braces | Tested | Preserve exact byte ranges. |
| Declaration, alias, and global | Partly tested | Index each form separately. |
| Recursive metadata | Pending | Use cycle detection and an expansion limit. |
| Missing debug metadata | Observed | Keep body and symbol evidence without source annotations. |
| Empty remark file | Observed | Treat it as a valid result. |
| Truncated artifact | Pending | Quarantine it and retain the diagnostic. |
| Multi-gigabyte module | Observed | Use bounded memory and 64-bit offsets. |
| Duplicate blob | Pending | Publish one content-addressed blob with several references. |
| Mutable incremental hardlink | Observed | Copy or reflink it into the immutable store. |
| Concurrent collectors | Pending | Use separate staging and one short catalog transaction. |
| Full disk or failed commit | Pending | Preserve visible records and remove orphan blobs later. |
| Corrupt existing blob | Pending | Quarantine it instead of overwriting a digest path. |
| Network filesystem | Pending | Detect unsupported locking or rename behavior and stop safely. |
| Excessive artifacts | Pending | Enforce byte, file-count, and parser-depth limits. |

## Platform coverage

| Platform | Status | Main remaining work |
| --- | --- | --- |
| Linux ELF, x86-64 | Observed | Versioned nightly contract tests. |
| `wasm32-unknown-unknown` | Observed | Final-module and symbol-size contracts. |
| macOS Mach-O, Apple silicon | Observed | dSYM paths and native-size attribution. |
| Windows MSVC | Pending | Process paths, response files, COFF, PDB, and dep-info drives. |
| Windows GNU | Pending | Wrapper execution, linker artifacts, and mixed path syntax. |
| Linux musl | Pending | Static linking and target tool availability. |
| Bare-metal custom target | Pending | Target specifications, missing linkers, and no standard library. |
