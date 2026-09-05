//! A streaming LLVM IR range scanner for research measurements.

use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemKind {
    Alias,
    Declaration,
    Definition,
    Global,
    Ifunc,
}

#[derive(Debug, Eq, PartialEq)]
struct IndexEntry {
    kind: ItemKind,
    name: Vec<u8>,
    start: u64,
    end: u64,
}

fn main() -> io::Result<()> {
    let path = env::args_os()
        .nth(1)
        .expect("usage: optic-research-ir-indexer PATH");
    let entries = scan(Path::new(&path))?;

    for kind in [
        ItemKind::Definition,
        ItemKind::Declaration,
        ItemKind::Alias,
        ItemKind::Ifunc,
        ItemKind::Global,
    ] {
        let count = entries.iter().filter(|entry| entry.kind == kind).count();
        println!("{kind:?}\t{count}");
    }

    if let Some(largest) = entries
        .iter()
        .filter(|entry| entry.kind == ItemKind::Definition)
        .max_by_key(|entry| entry.end - entry.start)
    {
        println!(
            "LargestDefinition\t{}\t{}\t{}",
            largest.end - largest.start,
            largest.start,
            String::from_utf8_lossy(&largest.name)
        );
    }

    Ok(())
}

fn scan(path: &Path) -> io::Result<Vec<IndexEntry>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut line = Vec::new();
    let mut offset = 0_u64;

    while read_line(&mut reader, &mut line)? != 0 {
        let start = offset;
        offset += line.len() as u64;

        if line.starts_with(b"define ") {
            let name = global_name(&line).unwrap_or_default();

            while !is_function_end(&line) {
                if read_line(&mut reader, &mut line)? == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "LLVM function reached the end of the file without a closing brace",
                    ));
                }
                offset += line.len() as u64;
            }

            entries.push(IndexEntry {
                kind: ItemKind::Definition,
                name,
                start,
                end: offset,
            });
        } else if line.starts_with(b"declare ") {
            entries.push(IndexEntry {
                kind: ItemKind::Declaration,
                name: global_name(&line).unwrap_or_default(),
                start,
                end: offset,
            });
        } else if line.starts_with(b"@") {
            let kind = if contains_token(&line, b" alias ") {
                ItemKind::Alias
            } else if contains_token(&line, b" ifunc ") {
                ItemKind::Ifunc
            } else {
                ItemKind::Global
            };

            entries.push(IndexEntry {
                kind,
                name: global_name(&line).unwrap_or_default(),
                start,
                end: offset,
            });
        }
    }

    Ok(entries)
}

fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<usize> {
    line.clear();
    reader.read_until(b'\n', line)
}

fn global_name(line: &[u8]) -> Option<Vec<u8>> {
    let start = line.iter().position(|byte| *byte == b'@')? + 1;

    if line.get(start) == Some(&b'"') {
        let mut cursor = start + 1;

        while cursor < line.len() {
            match line[cursor] {
                b'"' => return Some(line[start + 1..cursor].to_vec()),
                b'\\' => cursor += 2,
                _ => cursor += 1,
            }
        }

        return None;
    }

    let end = line[start..]
        .iter()
        .position(|byte| !is_identifier_byte(*byte))
        .map_or(line.len(), |length| start + length);

    Some(line[start..end].to_vec())
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'$' | b'.' | b'_')
}

fn contains_token(line: &[u8], token: &[u8]) -> bool {
    line.windows(token.len()).any(|window| window == token)
}

fn is_function_end(line: &[u8]) -> bool {
    let content = content_before_comment(line);
    trim_ascii(content) == b"}"
}

fn content_before_comment(line: &[u8]) -> &[u8] {
    let mut quoted = false;
    let mut cursor = 0;

    while cursor < line.len() {
        match line[cursor] {
            b'"' => quoted = !quoted,
            b'\\' if quoted => cursor += 1,
            b';' if !quoted => return &line[..cursor],
            _ => {}
        }
        cursor += 1;
    }

    line
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{content_before_comment, global_name, is_function_end};

    #[test]
    fn extracts_unquoted_and_quoted_names() {
        assert_eq!(
            global_name(b"define void @_Rexample() {"),
            Some(b"_Rexample".to_vec())
        );
        assert_eq!(
            global_name(br#"define void @"name with \22 escape"() {"#),
            Some(br#"name with \22 escape"#.to_vec())
        );
    }

    #[test]
    fn recognizes_only_a_top_level_closing_brace() {
        assert!(is_function_end(b"} ; end"));
        assert!(!is_function_end(br#"  call void asm "}", ""()"#));
    }

    #[test]
    fn ignores_semicolons_inside_strings() {
        assert_eq!(
            content_before_comment(br#"asm "a;b" ; comment"#),
            br#"asm "a;b" "#
        );
    }
}
