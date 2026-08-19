# Cargo Optic example

This example shows two compiler instances of one generic Rust function. The commands work in Fish,
Bash, and Zsh. They do not use shell variables.

## Install Cargo Optic

Run these commands from the root of the Optic repository:

```console
cargo +stable install --locked --path crates/cargo-optic --force
cd crates/cargo-optic/tests/fixtures/generic
```

The example requires the nightly toolchain. Install its compiler and LLVM libraries if they are
absent:

```console
rustup toolchain install nightly --component llvm-tools --component rustc-dev
```

## Show the example function

Run this command from the `generic` fixture directory:

```console
cargo +nightly optic show optic_mvp_kernel::outlined_sum \
  -p optic-mvp-app \
  --bin optic-mvp-app \
  --release \
  --source
```

The command builds the application and finds two instances:

- `outlined_sum::<u32, 4>`
- `outlined_sum::<u64, 8>`

Cargo Optic prints a complete `show` command for each instance. Copy one of these commands to show
the Rust source and optimized LLVM IR. You do not have to edit or combine IDs.

Run the first command again. Cargo checks the target and Optic reuses the completed capture. Add
`--fresh` to invoke rustc and create a new capture.

## Compare a source change

Keep one generated `show --instance` command for the old capture. This command continues to show
the old source and LLVM IR after you edit the fixture.

Open `kernel/src/lib.rs`. Replace the `fold` expression with this expression:

```rust
values.into_iter().fold(T::default(), |sum, value| {
    std::hint::black_box(sum + value)
})
```

Run the main `show` command again. Cargo detects the source change. Optic creates a new capture.
Copy one of the new instance commands to inspect the new LLVM IR.

Run the saved old instance command to read the old evidence. Cargo Optic reads `.optic` and does
not rebuild the instance.

Use the two instance IDs in this command:

```console
cargo +nightly optic compare \
  --before OLD_INSTANCE_ID \
  --after NEW_INSTANCE_ID
```

The command reports structural LLVM changes and incompatible capture dimensions.

Use this command to list all completed captures:

```console
cargo +nightly optic captures
```

Use this command to inspect one capture:

```console
cargo +nightly optic inspect --capture CAPTURE_ID
```

The result includes the request, compiler commands, wrappers, environment, and artifact stages.

## Select a different compiler output

The generated command shows optimized LLVM IR by default. Run this command to show LLVM IR before
optimization:

```console
cargo +nightly optic show optic_mvp_kernel::outlined_sum \
  -p optic-mvp-app \
  --bin optic-mvp-app \
  --release \
  --output llvm-pre-opt \
  --source
```

The generated instance commands keep the `--output` and `--source` options.

## Restore the source

Run this command to restore the original vectorized example:

```console
git restore kernel/src/lib.rs
```

## Manage the Optic store

Use these commands to inspect the store and verify its blobs:

```console
cargo +nightly optic status
cargo +nightly optic verify
```

Use these commands to remove one capture and its unreferenced blobs:

```console
cargo +nightly optic remove --capture CAPTURE_ID
cargo +nightly optic gc
```

## Remove the Optic cache

Run this command only when you want to remove all stored Optic evidence for this fixture:

```console
cargo +nightly optic clean
```

The command removes `.optic`. It keeps the Cargo `target` directory and its build artifacts.

If Cargo Optic rejects an old catalog version, run `clean` once. Then run the main `show` command
again.
