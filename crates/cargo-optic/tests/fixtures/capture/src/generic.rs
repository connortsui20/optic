//! Instantiates the capture fixture's generic functions from the selected binary target.
//!
//! Keeping the definitions local ensures the selected compiler invocation owns their concrete
//! monomorphizations.

use std::hint::black_box;
use std::ops::Add;

// Request separate emitted bodies for both generic instances.
#[inline(never)]
fn outlined_kernel<T: Copy>(value: T) -> T {
    black_box(value)
}

// Make the nested generic function available for compiler instance collection.
fn nested_kernel<T>(value: T) -> T
where
    T: Add<Output = T> + From<u8>,
{
    fn chunk<T>(value: T) -> T
    where
        T: Add<Output = T> + From<u8>,
    {
        value + T::from(1)
    }

    chunk(value)
}

fn main() {
    black_box((
        outlined_kernel::<u32>(black_box(42)),
        outlined_kernel::<u64>(black_box(42)),
        nested_kernel::<u16>(black_box(42)),
    ));
}
