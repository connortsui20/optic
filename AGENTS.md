# Contributing to Cargo Optic

Keep code simple enough that a contributor can follow each workflow in one pass.

- Read [the Rust style reference](docs/rust-style.md) before designing or changing Rust code.
- Follow the active contracts on the `planning` branch. Historical research is not additional scope.
- Preserve the seven product crates. Add internal abstractions only for a current contract or caller.
- Document public fields, variants, entry points, and non-obvious correctness arguments.
- Keep unsupported inputs explicit. Do not add compatibility, recovery, or platform fallback layers.
- Add focused tests for changed behavior and practical regressions.
- Keep Cargo integration tests isolated from user configuration and process-global environment changes.
- Run formatting, Clippy, rustdoc, and the affected tests before handing off a change.
- Use independent correctness and Rust-style review before merging an integrated product change.

The complete MVP uses one integration branch with cached capture/list/find before narrow show.
Persist decisions and checkpoint evidence on `planning` before dependent implementation starts.

Use `bash scripts/check-format.sh` to include the standalone driver in formatting checks.
Use `cargo test --workspace` for the repository suite and `bash scripts/check-install.sh` for archives
and installed-product checks. Read [the testing guide](docs/testing.md) when adding a regression.
