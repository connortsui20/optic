mod producer;

use std::hint::black_box;

use optic_research_kernel::{
    Accumulate, Engine, GENERATED_BY_BUILD_SCRIPT, GENERATED_ENV, NAMED_STATIC, alpha,
    async_identity, beta, café, capture, const_bool, const_char, const_signed, dyn_dispatch,
    inlined_sum, outlined_sum, projected, spaced_value, unused_const, unused_type,
};
use optic_research_passthrough_macro::optic_passthrough;

#[optic_passthrough]
#[inline(never)]
fn macro_annotated(value: u64) -> u64 {
    black_box(value)
}

fn main() {
    let engine = Engine::<u64>::new();

    black_box(producer::cross_cgu(black_box(9)));
    black_box(outlined_sum::<u64, 8>([1; 8]));
    black_box(outlined_sum::<u32, 4>([1; 4]));
    black_box(inlined_sum::<u64, 8>([1; 8]));
    black_box(inlined_sum::<u32, 4>([1; 4]));
    black_box(engine.inherent::<8>([1; 8]));
    black_box(engine.accumulate::<4>([1; 4]));
    black_box(unused_type::<u32>(1));
    black_box(unused_type::<u64>(2));
    black_box(unused_const::<4>(3));
    black_box(unused_const::<8>(4));
    black_box(alpha::duplicate::<u64>(5));
    black_box(beta::duplicate::<u64>(6));
    black_box(projected::<Engine<u64>>(7));
    black_box(café::<u64>(8));
    black_box(const_bool::<true>(8));
    black_box(const_char::<'ß'>(8));
    black_box(const_signed::<-3>(8));
    black_box(dyn_dispatch(&10_u64));
    black_box(spaced_value(10));
    black_box(NAMED_STATIC);
    let function: fn([u64; 8]) -> u64 = outlined_sum::<u64, 8>;
    black_box(function([1; 8]));
    black_box(macro_annotated(9));
    black_box(GENERATED_BY_BUILD_SCRIPT);
    black_box(GENERATED_ENV);
    drop(black_box(async_identity::<u64>(1)));
    black_box(capture::<u64>(1)());
}
