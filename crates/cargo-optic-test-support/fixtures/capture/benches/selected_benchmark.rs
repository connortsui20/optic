//! Supplies a distinct concrete instance for explicit benchmark selection.
//!
//! Cargo disables the test harness for this target, so the benchmark uses an ordinary main function.

#[inline(never)]
fn benchmark_instance<T: Copy>(value: T) -> T {
    std::hint::black_box(value)
}

fn main() {
    std::hint::black_box(benchmark_instance(42_u64));
}
