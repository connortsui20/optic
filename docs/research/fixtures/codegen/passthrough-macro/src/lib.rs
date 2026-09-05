//! A dependency-free procedural macro for host compiler experiments.

use proc_macro::TokenStream;

/// Returns the annotated item without changes.
#[proc_macro_attribute]
pub fn optic_passthrough(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    item
}
