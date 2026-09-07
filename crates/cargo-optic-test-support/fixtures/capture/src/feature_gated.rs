//! Provides a binary target that requires an explicit Cargo feature.
//!
//! Cargo must enable the `gated` feature before this target can be selected for capture.

fn main() {
    std::hint::black_box(capture_fixture::captured_value());
}
