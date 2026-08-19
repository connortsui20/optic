//! Instantiates two concrete versions of the acceptance-test kernel.
//!
//! Cargo Optic must distinguish both versions and retain a body for each version.

use std::hint::black_box;

use optic_mvp_kernel::{
    BUILD_DATA, BUILD_ENV, GenericKernel, ReexportedKernel, inline_add_one, outlined_sum,
};

fn main() {
    let u32_sum = outlined_sum::<u32, 4>(black_box([1, 2, 3, 4]));
    let u64_sum = outlined_sum::<u64, 8>(black_box([1, 2, 3, 4, 5, 6, 7, 8]));
    let identity = ReexportedKernel::identity(42_u64);
    let generic = GenericKernel::new(42_u16);
    let incremented = inline_add_one(42_u64);

    black_box((
        u32_sum,
        u64_sum,
        identity,
        generic,
        incremented,
        BUILD_DATA,
        BUILD_ENV,
    ));
}
