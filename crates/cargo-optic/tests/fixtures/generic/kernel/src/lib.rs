//! Generic functions used by the Cargo Optic acceptance test.
//!
//! The application package instantiates this path-dependency function with two element types.

/// Sums an array through a standalone compiler instance.
///
/// The acceptance test uses `#[inline(never)]` so each instance keeps a standalone LLVM body.
#[inline(never)]
pub fn outlined_sum<T, const LENGTH: usize>(values: [T; LENGTH]) -> T
where
    T: Copy + Default + std::ops::Add<Output = T>,
{
    values
        .into_iter()
        .fold(T::default(), |sum, value| sum + value)
}

mod implementation {
    /// A type whose public path differs from its canonical definition path.
    pub struct ReexportedKernel;

    impl ReexportedKernel {
        /// Returns one value through a standalone generic compiler instance.
        #[inline(never)]
        pub fn identity<T>(value: T) -> T {
            std::hint::black_box(value)
        }
    }
}

pub use implementation::ReexportedKernel;
