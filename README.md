# Cargo Optic

Cargo Optic captures the concrete function instances that Rust compiles and shows their source or
optimized LLVM IR. A concrete instance is a function with its generic arguments resolved. The tool
finds instances from a selected Cargo target, including dependency instances used by that target. It
does not index every function in the source tree.

Cargo Optic is experimental and supports Linux and macOS.

## Install

Install from a checkout with Git and rustup. The [toolchain file][toolchain] selects the supported
compiler and required components:

```console
git clone https://github.com/connortsui20/optic.git
cd optic
cargo install --path crates/cargo-optic --locked
```

Capturing evidence requires the matching `rustc-dev` and `llvm-tools` components after installation.
Commands that read stored evidence do not require the compiler used for capture.

## Try it

From this checkout, capture the `cargo-optic-records` library:

```console
cargo optic capture -p cargo-optic-records --lib --release
```

The first capture builds a compiler driver and caches it for later use. Repeating the command lets
Cargo check whether the capture is still fresh. A fresh capture prints `Reused` with its original
ID.

Use the ID after `Captured` or `Reused` as `CAPTURE_REF`:

```console
cargo optic find --capture CAPTURE_REF CaptureId::generate
```

Use the value from the result's `Reference` line as `INSTANCE_REF`:

```console
cargo optic show --instance INSTANCE_REF --output source
cargo optic show --instance INSTANCE_REF --output llvm
```

Both commands read captured evidence. Later source edits do not change that evidence.

For another project, run Optic from its workspace. Select its package, target, and profile
explicitly. The [usage guide][usage] covers target selection, search, cache behavior, and evidence
limits.

```console
cargo optic --help
```

## Current limits

- The compiler driver uses unstable rustc APIs. Other compiler versions can fail during driver
  compilation or provide no LLVM evidence.
- Optimized LLVM output supports nonincremental builds with no LTO or local ThinLTO. Cargo's
  `lto = "thin"` is cross-crate ThinLTO and does not provide LLVM output.
- Some instances have no available source or standalone LLVM body. Capture and search remain useful
  when evidence is unavailable.
- Captures require one writer per workspace. Driver provisioning must also run serially within each
  Cargo home.
- Stored data and Rust APIs have no compatibility or migration guarantees.

The usage guide lists the [supported configurations and evidence limits][limits].

## Library

The `cargo-optic-api` package exposes the `optic` library for capture, search, and stored evidence
reads. The [architecture guide][architecture] explains the crate boundaries and data flow.

## License

Licensed under either the [MIT license][mit] or the [Apache License 2.0][apache], at your option.

[toolchain]: https://github.com/connortsui20/optic/blob/main/rust-toolchain.toml
[usage]: https://github.com/connortsui20/optic/blob/main/docs/usage.md
[limits]: https://github.com/connortsui20/optic/blob/main/docs/usage.md#supported-configurations-and-evidence-limits
[architecture]: https://github.com/connortsui20/optic/blob/main/docs/architecture.md
[mit]: https://github.com/connortsui20/optic/blob/main/LICENSE-MIT
[apache]: https://github.com/connortsui20/optic/blob/main/LICENSE-APACHE
