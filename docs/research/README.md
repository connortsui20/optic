# Optic research

> Historical research or broader design, not the current implementation contract. The
> [active MVP plan](../design/PLAN.md) defines scope, implementation order, and required tests.

Optic records compiler evidence from real Cargo builds. It lets users and tools inspect and compare
concrete Rust instances across compiler stages.

This directory preserves research from the separate experimental `prototype` branch.
The [`design plan`](../design/PLAN.md) defines the current MVP contract.

No document here is required for a review of `main`.

## Research summary

[`core.md`](core.md) summarizes the historical experiments and their compiler background.
Its implemented capabilities describe `prototype`, not the current MVP.

It covers:

- What Optic can implement.
- The small amount of compiler background needed for the design.
- The main findings from the compiler experiments.
- The experimental product boundary.
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
Use these fixtures only to investigate historical findings.
For current product verification, use [the testing guide](../testing.md) and the fixtures on `main`.
