#!/usr/bin/env bash
# Check both Cargo modules and the runtime-compiled standalone driver.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo fmt --all -- --check
rustfmt --edition 2024 --check crates/cargo-optic-compiler/rustc-driver/main.rs
rustfmt --edition 2024 --check crates/cargo-optic-compiler/tests/fixtures/stage-proof.rs
