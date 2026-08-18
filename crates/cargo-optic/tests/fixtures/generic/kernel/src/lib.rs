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
