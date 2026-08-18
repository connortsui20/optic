//! Indexes function bodies in textual LLVM modules with bounded memory.
//!
//! [`scan`] records byte ranges and raw symbols without retaining a complete module. Callers can
//! later seek directly to one body, including in multi-gigabyte LTO artifacts.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use rustc_demangle::try_demangle;

use crate::{BodyRange, Error, Result};

pub(crate) fn scan(path: &Path) -> Result<Vec<BodyRange>> {
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open",
        path: path.to_owned(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut bodies = Vec::new();
    let mut line = Vec::new();
    let mut offset = 0_u64;

    while read_line(&mut reader, &mut line, path)? != 0 {
        let start = offset;
        offset += line.len() as u64;

        if !line.starts_with(b"define ") {
            continue;
        }

        let Some(symbol) = global_name(&line) else {
            return Err(Error::InvalidLlvm {
                path: path.to_owned(),
                message: "function definition does not contain a global symbol".to_owned(),
            });
        };

        while !is_function_end(&line) {
            if read_line(&mut reader, &mut line, path)? == 0 {
                return Err(Error::InvalidLlvm {
                    path: path.to_owned(),
                    message: "function reached the end of the file without a closing brace"
                        .to_owned(),
                });
            }
            offset += line.len() as u64;
        }

        let demangled =
            try_demangle(&symbol).map_or_else(|_| symbol.clone(), |name| format!("{name:#}"));
        bodies.push(BodyRange {
            raw_symbol: symbol,
            demangled,
            start,
            end: offset,
        });
    }

    Ok(bodies)
}

fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>, path: &Path) -> Result<usize> {
    line.clear();
    reader
        .read_until(b'\n', line)
        .map_err(|source| Error::Filesystem {
            operation: "read",
            path: path.to_owned(),
            source,
        })
}

fn global_name(line: &[u8]) -> Option<String> {
    let start = line.iter().position(|byte| *byte == b'@')? + 1;

    if line.get(start) == Some(&b'"') {
        let mut cursor = start + 1;

        while cursor < line.len() {
            match line[cursor] {
                b'"' => {
                    return Some(String::from_utf8_lossy(&line[start + 1..cursor]).into_owned());
                }
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

    Some(String::from_utf8_lossy(&line[start..end]).into_owned())
}

const fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'$' | b'.' | b'_')
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
            Some("_Rexample".to_owned())
        );
        assert_eq!(
            global_name(br#"define void @"name with \22 escape"() {"#),
            Some(r"name with \22 escape".to_owned())
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
