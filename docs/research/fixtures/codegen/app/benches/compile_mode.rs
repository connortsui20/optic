use std::hint::black_box;

use optic_research_kernel::outlined_sum;

fn main() {
    black_box(outlined_sum::<usize, 3>([1, 2, 3]));
}
