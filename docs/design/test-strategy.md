# Test strategy

The tests protect product claims and architecture boundaries. The test infrastructure stays smaller
than the code that it protects.

## Principles

- Test behavior through the narrowest public boundary that proves the claim.
- Use a real Cargo process for Cargo and rustc integration.
- Isolate each process test from user configuration and shared target state.
- Keep fixtures small enough that a reader can understand the expected compiler output.
- Use semantic assertions for text output. Do not use broad snapshots.
- Test a documented error when unsupported input is part of the public boundary.
- Do not create tests for unsupported environments only to make internal fallback code permanent.
- Add a regression test with each defect correction when the failure is practical to reproduce.

## Test levels

### Unit tests

Unit tests own pure invariants in one crate. They cover record construction, validation, identifier
parsing, request validation, and store-path rules.

A unit test does not invoke Cargo when a direct value can prove the same invariant.

### Subsystem integration tests

Compiler integration tests invoke real Cargo and rustc processes. Store integration tests use a
real temporary filesystem.

These tests own process selection, wrapper behavior, driver protocol exchange, publication, and
bounded durable reads.

### Product API tests

The `optic` tests cross the application API boundary. They own workflow composition and typed error
mapping across compiler, records, and store crates.

### CLI tests

The CLI tests invoke the built `cargo-optic` binary as an external Cargo subcommand. They own exit
status, standard output, standard error, argument behavior, and follow-up references.

### Package test

The release phase installs Cargo Optic outside the source workspace. It then runs the documented
workflow against the fixture package.

This test enters the suite only when package metadata and external installation become release
contracts.

## Hermetic environment

Each process test owns one temporary root with these paths:

```text
test-root/
|-- cargo-home/
|-- store/
|-- target/
+-- workspace/
```

The harness sets the current directory to `workspace/`. It points Cargo and Cargo Optic state at
the temporary paths.

The default command removes these environment variables:

- `RUSTC`.
- `CARGO_BUILD_RUSTC`.
- `RUSTC_WRAPPER`.
- `RUSTC_WORKSPACE_WRAPPER`.
- `CARGO_BUILD_RUSTC_WRAPPER`.
- `CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER`.
- `RUSTC_BOOTSTRAP`.

A test can set one removed value when that value is the test input. The harness must not change the
selected rustup toolchain or hide the matching `rustc-dev` and LLVM components.

## Fixture policy

Use one default fixture until a behavior needs an incompatible project shape. The fixture includes
only source that supports a named test dimension.

Keep checked-in fixture files when Cargo must observe real paths or manifests. Generate a file in
the test when its exact contents are the test input.

Do not add a fixture for an unsupported platform case. First approve the platform behavior in the
planning documents.

## Contract matrix

The current matrix records these dimensions:

| Area | Required cases | Owning level |
| --- | --- | --- |
| Request selection | Package, target, profile, and features. | Unit and API. |
| Compiler selection | Default rustc, configured rustc, and environment rustc. | Compiler integration. |
| Wrapper policy | No wrapper and one disabled wrapper with warnings. | Compiler integration. |
| Capture publication | Success and each pre-publication error. | API and store integration. |
| Capture listing | Empty and populated stores. | API and CLI. |
| Instance collection | Non-generic and multiply instantiated generic functions. | Compiler integration. |
| Instance lookup | Exact name, substring, deterministic order, and limit. | API and CLI. |
| Durable input | Valid, malformed, unsupported-version, and oversized records. | Records and store. |
| Process protocol | Matching writer and reader, wrong magic, and wrong version. | Compiler integration. |
| Output | Exit status, stdout result, stderr warnings, and closed stdout. | CLI. |

Update this matrix before or with a product contract. A test name must state the behavior, not the
internal function that it calls.

## CI matrix

The initial workflow contains these jobs:

| Job | Host | Command |
| --- | --- | --- |
| Format | Linux | `cargo fmt --all -- --check` |
| Clippy | Linux | `cargo clippy --workspace --all-targets -- -D warnings` |
| Documentation | Linux | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` |
| Test | Linux and macOS | `cargo test --workspace` |

The workflow uses the repository toolchain file. It does not select a second compiler version.

Do not add retries for a deterministic test. Diagnose the error or remove accidental external
state. Do not add CI caching until job duration is a demonstrated development problem.

## Future feature tests

### Captured source

The source slice covers an available snapshot, missing source, a changed checkout after capture,
an invalid range, and a path outside approved roots.

### Exact LLVM IR

The LLVM slice covers an exact body, an optimized-away body, equal display text with different raw
symbols, an invalid range, and a module larger than one requested body.

### Capture reuse

The reuse slice does this sequence:

1. Capture one target and record a selected-target rustc invocation.
2. Repeat the same request and receive the same capture ID without another rustc invocation.
3. Change tracked source and receive a new capture ID from a new rustc invocation.
4. Use `--fresh` and receive a new capture ID when prior analysis is fresh.
5. Repeat a matching request and make sure that the cached driver file does not change.

The test asks Cargo to decide freshness. It does not assert an Optic reconstruction of Cargo's
fingerprint inputs.

## Deferred test systems

The MVP does not need these systems:

- A declarative scenario language.
- Snapshot approval tooling.
- Property-based generation.
- Continuous fuzzing.
- Mutation tests.
- Performance benchmarks.
- Distributed fixtures.
- Windows or cross-target CI.

A later plan can add one of these systems after a concrete defect or workload demonstrates the
need.
