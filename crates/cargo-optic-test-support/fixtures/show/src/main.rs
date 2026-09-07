//! Exercises local, external, and generated definitions in one selected binary.
//!
//! Separate source modules keep source-file ownership and multiple codegen units observable.

mod source_items;

#[path = "source with spaces.rs"]
mod spaced;

use std::hint::black_box;

fn main() {
    let value = black_box(7_u64);
    let widget = source_items::Widget(value);

    black_box((
        source_items::ordinary(value),
        widget.method(value),
        source_items::generic(black_box(1_u32)),
        source_items::generic(value),
        source_items::generated(value),
        show_dependency::external_generic(value),
        spaced::unicode_function(value),
    ));
}
