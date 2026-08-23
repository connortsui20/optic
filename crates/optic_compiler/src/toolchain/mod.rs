//! Identifies the compiler that Cargo will use without changing the build.
//!
//! [`selected_rustc`] follows Cargo's environment and configuration rules to choose an executable.
//! [`inspect_rustc`] then records the stable identity reported by that executable. Keeping selection
//! and inspection separate makes it clear that neither operation modifies the captured build's
//! wrappers, flags, or environment.

mod identity;
pub(crate) use identity::inspect_rustc;

mod selection;
pub(crate) use selection::selected_rustc;
