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

/// Adds one through a nested helper that the optimizer can fully inline.
#[inline(always)]
pub fn inline_add_one<T>(value: T) -> T
where
    T: From<u8> + std::ops::Add<Output = T>,
{
    #[inline(always)]
    fn chunk<T>(value: T) -> T
    where
        T: From<u8> + std::ops::Add<Output = T>,
    {
        value + T::from(1)
    }

    chunk(value)
}

/// A non-Rust compiler input used to verify Cargo-observed freshness.
pub const BUILD_DATA: &[u8] = include_bytes!("build-data.txt");

/// A compiler environment input used to verify Cargo-observed freshness.
pub const BUILD_ENV: Option<&str> = option_env!("OPTIC_TEST_VALUE");
