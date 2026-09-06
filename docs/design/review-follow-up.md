# MVP review follow-up

The user requested a new PR for two reproduced regressions in `main` at `bae4ca0`, plus related cleanup.
The completed MVP remains the baseline. This follow-up does not add features or compatibility layers.

## Fixes and ownership

- The integration owner fixes Cargo executable discovery and adds isolated process regressions.
- A compiler worker replaces repeated source-prefix scans with rustc's existing line index.
- A separate reviewer audits nearby capture, evidence, and process boundaries for concrete defects.
- The integration owner records any additional defect before implementing its correction.

Preserve explicit `CARGO` selection and the selected executable's invocation path. PATH lookup must
skip non-executable entries and retain ordinary symlink and relative-directory behavior. Do not add
Windows support, shell invocation, compiler overrides, or wrapper-composition machinery.

Source line lookup must use the pinned compiler's source map and preserve one-based normalized line
numbers. Test first and later lines, multiple files, and repeated concrete instances. Reproduce the
large-file cost without a machine-dependent timing assertion or another source-line cache.

## Verification and delivery

- [ ] Reproduce each reported regression and add focused coverage.
- [ ] Complete both fixes under the full Rust style rules.
- [ ] Audit related code and resolve any additional reproducible issues found.
- [ ] Run workspace tests, standalone formatting, Clippy, and public/private rustdoc.
- [ ] Run archive installation, the external API consumer, and self-hosting verification.
- [ ] Complete independent correctness and Rust-style review of the final diff.
- [ ] Open the corrective PR and verify Linux/macOS CI on its final revision.
- [ ] Record results and the PR link on `planning`.

Use the existing isolated fixture framework. Never change the parent process environment, uninstall
toolchain components, or modify user Cargo configuration to reproduce errors. Registry DNS failure
is a verification blocker, not a reason to change product code or disable archive verification.

The corrective PR remains open for review at handoff. No registry publication is authorized.
