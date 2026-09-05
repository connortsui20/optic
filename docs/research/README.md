# Optic research

Optic records compiler evidence from real Cargo builds. It lets users and tools inspect and compare
concrete Rust instances across compiler stages.

This directory preserves the research that informed the current prototype. The
[`design plan`](../design/PLAN.md) defines current product behavior.

The research has one required document and one optional reference section.

## Required reading

Read [`core.md`](core.md). It explains the findings that still control the current architecture.

It covers:

- What Optic can implement.
- The small amount of compiler background needed for the design.
- The main findings from the compiler experiments.
- The implemented product boundary.
- The remaining research areas.

## Design reference

The [`reference/`](reference/) directory preserves the original detailed design. It is historical
research, not the current product contract.

- [`capture.md`](reference/capture.md) describes compiler artifacts, evidence channels, Cargo
  behavior, and the original capture-fidelity experiments.
- [`implementation.md`](reference/implementation.md) describes records, collection, storage,
  identity, source capture, comparisons, and unimplemented ideas.
- [`test-matrix.md`](reference/test-matrix.md) preserves the original contract-test matrix.

## Fixtures

The [`fixtures/`](fixtures/) directory contains the programs and scripts used in the experiments.
Use these fixtures to reproduce the historical findings. Use the product fixture on the prototype
branch for current manual validation.
