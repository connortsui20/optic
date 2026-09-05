//! Supplies a distinct concrete instance for explicit example selection.
//!
//! The selected invocation owns this generic function's emitted body.

#[inline(never)]
fn example_instance<T: Copy>(value: T) -> T {
    std::hint::black_box(value)
}

fn main() {
    std::hint::black_box(example_instance(42_u64));
}
