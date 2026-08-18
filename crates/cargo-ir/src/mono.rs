//! Parses rustc's concrete monomorphization inventory.
//!
//! A [`MonoItem`] records the compiler-owned display name and every codegen unit placement printed
//! by `-Z print-mono-items=yes`.

use serde::{Deserialize, Serialize};

/// One concrete function selected for monomorphization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MonoItem {
    /// The concrete Rust function name printed by rustc.
    pub name: String,

    /// The codegen units in which rustc placed the item.
    pub codegen_units: Vec<String>,
}

pub(crate) fn parse(stderr: &str) -> Vec<MonoItem> {
    stderr.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<MonoItem> {
    let item = line.strip_prefix("MONO_ITEM fn ")?;
    let (name, placements) = item.split_once(" @@ ")?;
    let codegen_units = placements
        .split_whitespace()
        .filter_map(|placement| placement.split_once('[').map(|(unit, _)| unit.to_owned()))
        .collect();

    Some(MonoItem {
        name: name.to_owned(),
        codegen_units,
    })
}

#[cfg(test)]
mod tests {
    use super::{MonoItem, parse_line};

    #[test]
    fn parses_all_codegen_unit_placements() {
        let line =
            "MONO_ITEM fn crate::kernel::<u64> @@ crate-cgu.0[Internal] crate-cgu.1[External]";

        assert_eq!(
            parse_line(line),
            Some(MonoItem {
                name: "crate::kernel::<u64>".to_owned(),
                codegen_units: vec!["crate-cgu.0".to_owned(), "crate-cgu.1".to_owned()],
            })
        );
    }
}
