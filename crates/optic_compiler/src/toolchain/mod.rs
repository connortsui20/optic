//! Identifies the compiler that Cargo will use without changing the build.
//!
//! [`rustc_invocation`] follows Cargo's environment and configuration rules to select the complete
//! wrapper chain. [`inspect_rustc`] records the identity reported through that same chain. Neither
//! operation modifies the captured build's wrappers, flags, or environment.

mod identity;
pub(crate) use identity::inspect_rustc;

mod selection;
pub(crate) use selection::rustc_invocation;
