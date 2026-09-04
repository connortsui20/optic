use std::hint::black_box;
use std::ops::Add;

trait Kernel {
    fn trait_kernel(&self) -> u64;
}

struct LocalKernel;

impl Kernel for LocalKernel {
    #[inline(never)]
    fn trait_kernel(&self) -> u64 {
        black_box(42)
    }
}

// Request standalone definitions for exact and substring search subjects.
#[inline(never)]
fn generic_kernel<T: Copy>(value: T) -> T {
    black_box(value)
}

#[inline(never)]
fn generic_kernel_suffix(value: u64) -> u64 {
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

#[inline(never)]
fn SCOPED_KERNEL(value: u64) -> u64 {
    black_box(value)
}

fn main() {
    black_box((
        generic_kernel(1_u32),
        generic_kernel(1_u64),
        generic_kernel_suffix(1_u64),
        nested_kernel(1_u16),
        LocalKernel.trait_kernel(),
        SCOPED_KERNEL(1_u64),
    ));
}
