# Motivation

This document describes the problem that led to Cargo Optic. The [product overview](overview.md)
explains the current goals and features. The detailed product design is in
[`cargo-optic.md`](cargo-optic.md).

## The investigation problem

Performance-sensitive Rust code often requires inspection of compiler output. Useful evidence can
include LLVM IR, optimization remarks, source, assembly, and other compiler stages.

Existing tools can show many artifacts. The user must still connect each artifact to the correct
Rust function and build.

Generic functions, codegen units, inlining, and LTO make this connection difficult. A short name or
display path cannot prove compiler identity.

Cargo Optic starts with one real Cargo target. It finds concrete compiler instances and shows the
evidence that belongs to each instance.

## Function identity

A full Rust path identifies a definition, but it does not always identify one compiler instance. A
generic definition can produce many concrete instances.

Another crate can create an instance. Rustc can place one instance in several codegen units, and
LLVM can remove its standalone body.

Cargo Optic stores definitions, instances, placements, bodies, declarations, and aliases as
separate facts. It reports ambiguity instead of selecting a candidate without exact evidence.

The exact-version rustc driver records each instance and raw LLVM symbol. The LLVM index joins a
body only through that exact raw symbol.

## Compiler stages

The phrase `the generated code` does not identify one compiler form. A function can change between
pre-optimization LLVM, optimized LLVM, and ThinLTO stages.

Each stage answers a different question. Pre-optimization LLVM shows rustc's LLVM input, while
optimized LLVM shows later transformations.

Optimization remarks explain selected LLVM pass decisions. They complement the saved IR, but they
do not replace it.

The current prototype supports optimized LLVM IR, pre-optimization LLVM IR, and optimization
remarks. MIR, assembly, and object evidence remain outside the current product boundary.

## Persistent evidence

Compilation is often the most expensive part of an investigation. Artifact discovery and instance
lookup also repeat after each source change.

Cargo Optic stores immutable completed captures below `.optic/store`. Later queries reuse this
evidence after Cargo validates the selected target's freshness.

The store contains observations, not normal build products. Cargo keeps responsibility for build
freshness and reusable dependency artifacts.

A failed ingestion can retain validated post-compilation evidence. A matching request resumes that
work after Cargo validates freshness.

## Interfaces

People and agents use the same `cargo optic` command. Plain text is the default format, and
versioned JSON Lines events support programs.

The `.optic` directory is a shared workspace store, not a session. Each query uses an explicit
capture or instance ID.

The typed `cargo_optic::Application` API exposes the same workflows. A future TUI can use this API,
but the prototype has no TUI or client context.

## Product boundary

Cargo Optic gives insight into compiler output from real Rust builds. Concrete compiler instances
remain the main unit for lookup and comparison.

The product is not a build system, profiler, benchmark runner, or general compiler debugger. It
does not infer performance from static LLVM counts.

Cargo Optic uses the stable, beta, or nightly rustc that the workspace selects. The selected
compiler requires matching `rustc-dev` and `llvm-tools` components.
