use std::hint::black_box;

use optic_research_kernel::outlined_sum;

#[test]
fn calls_a_generic_function() {
    assert_eq!(black_box(outlined_sum::<u16, 2>([1, 2])), 3);
}
