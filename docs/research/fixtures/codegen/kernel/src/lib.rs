//! Generic functions for the compiler research fixtures.

use std::hint::black_box;
use std::marker::PhantomData;

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[path = "space name.rs"]
mod spaced;

pub use spaced::spaced_value;

/// Computes one value from an array.
///
/// ```
/// use optic_research_kernel::outlined_sum;
///
/// assert_eq!(outlined_sum::<u64, 3>([1, 2, 3]), 6);
/// ```
///
/// ```compile_fail
/// use optic_research_kernel::outlined_sum;
///
/// let _: String = outlined_sum::<u64, 1>([1]);
/// ```
#[inline(never)]
pub fn outlined_sum<T, const N: usize>(values: [T; N]) -> T
where
    T: Copy + Default + std::ops::Add<Output = T>,
{
    values
        .into_iter()
        .fold(T::default(), |sum, value| sum + value)
}

/// Computes one value from an array and permits inlining.
#[inline(always)]
pub fn inlined_sum<T, const N: usize>(values: [T; N]) -> T
where
    T: Copy + Default + std::ops::Add<Output = T>,
{
    values
        .into_iter()
        .fold(T::default(), |sum, value| sum + value)
}

/// Defines a generic trait method with a const argument.
pub trait Accumulate<T> {
    /// Computes one value from an array.
    fn accumulate<const N: usize>(&self, values: [T; N]) -> T;
}

/// Selects the implementation type for generic method fixtures.
pub struct Engine<T>(PhantomData<T>);

impl<T> Engine<T> {
    /// Creates an engine.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for Engine<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine<u64> {
    /// Defines an inherent generic method.
    #[inline(never)]
    pub fn inherent<const N: usize>(&self, values: [u64; N]) -> u64 {
        outlined_sum(values)
    }
}

impl Accumulate<u32> for Engine<u64> {
    #[inline(never)]
    fn accumulate<const N: usize>(&self, values: [u32; N]) -> u32 {
        outlined_sum(values)
    }
}

/// Creates a generic future.
pub async fn async_identity<T: Copy>(value: T) -> T {
    black_box(value)
}

/// Creates a closure that captures a generic value.
pub fn capture<T: Copy>(value: T) -> impl Fn() -> T {
    move || black_box(value)
}

/// Uses a type argument that does not affect the function signature or body.
#[inline(never)]
pub fn unused_type<T>(value: u64) -> u64 {
    black_box(value)
}

/// Uses a const argument that does not affect the function body.
#[inline(never)]
pub fn unused_const<const N: usize>(value: u64) -> u64 {
    black_box(value)
}

/// Defines duplicate leaf names in separate modules.
pub mod alpha {
    /// Returns a generic value.
    #[inline(never)]
    pub fn duplicate<T: Copy>(value: T) -> T {
        std::hint::black_box(value)
    }
}

/// Defines duplicate leaf names in separate modules.
pub mod beta {
    /// Returns a generic value.
    #[inline(never)]
    pub fn duplicate<T: Copy>(value: T) -> T {
        std::hint::black_box(value)
    }
}

/// Describes a type through an associated type projection.
pub trait HasItem {
    /// Selects the projected type.
    type Item: Copy;
}

impl HasItem for Engine<u64> {
    type Item = u32;
}

/// Returns a value selected through an associated type projection.
#[inline(never)]
pub fn projected<T: HasItem>(value: T::Item) -> T::Item {
    black_box(value)
}

/// Uses a Unicode identifier in the Rust item path.
#[inline(never)]
pub fn café<T: Copy>(value: T) -> T {
    black_box(value)
}

/// Returns a value with a boolean const argument in its identity.
#[inline(never)]
pub fn const_bool<const VALUE: bool>(value: u64) -> u64 {
    black_box(value)
}

/// Returns a value with a character const argument in its identity.
#[inline(never)]
pub fn const_char<const VALUE: char>(value: u64) -> u64 {
    black_box(value)
}

/// Returns a value with a signed const argument in its identity.
#[inline(never)]
pub fn const_signed<const VALUE: i8>(value: u64) -> u64 {
    black_box(value)
}

/// Supplies one method for dynamic dispatch experiments.
pub trait DynValue {
    /// Returns the stored value.
    fn value(&self) -> u64;
}

impl DynValue for u64 {
    fn value(&self) -> u64 {
        *self
    }
}

/// Calls a method through a trait object.
#[inline(never)]
pub fn dyn_dispatch(value: &dyn DynValue) -> u64 {
    value.value()
}

/// Provides a named static item for mono-item inventory experiments.
pub static NAMED_STATIC: u64 = 0x5678;

/// Exports a function without a Rust v0 symbol.
#[unsafe(export_name = "optic_research_exported")]
pub extern "C" fn exported(value: u64) -> u64 {
    black_box(value)
}
