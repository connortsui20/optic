//! Provides whole functions, a method, shared generic source, and a macro expansion.
//!
//! Every subject has an emitted body so source tests can select a concrete compiler instance.

#[inline(never)]
/// Doubles the input with an operation that LLVM can simplify.
pub fn ordinary(value: u64) -> u64 {
    let doubled = value.wrapping_add(value);

    doubled.wrapping_add(0)
}

/// Supplies a receiver for a captured method.
pub struct Widget(
    /// The value included in the method result.
    pub u64,
);

impl Widget {
    #[inline(never)]
    /// Combines the receiver and input in a whole method body.
    pub fn method(&self, value: u64) -> u64 {
        self.0.wrapping_mul(3).wrapping_add(value)
    }
}

#[inline(never)]
/// Shares one source span between two concrete instances.
pub fn generic<T: Copy>(value: T) -> T {
    std::hint::black_box(value)
}

macro_rules! define_generated {
    () => {
        #[inline(never)]
        /// Supplies a generated definition whose call site cannot stand in for its source.
        pub fn generated(value: u64) -> u64 {
            std::hint::black_box(value.wrapping_mul(5))
        }
    };
}

define_generated!();
