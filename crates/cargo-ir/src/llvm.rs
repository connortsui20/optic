//! Indexes symbol records in textual LLVM modules with bounded memory.
//!
//! [`scan`] records function definitions, declarations, and aliases without retaining a complete
//! module. Callers can later seek directly to one record, including in multi-gigabyte LTO
//! artifacts.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use rustc_demangle::try_demangle;

use crate::{AliasTarget, BodyRange, Error, LlvmAlias, LlvmDeclaration, Result};

pub(crate) struct ModuleIndex {
    pub(crate) bodies: Vec<BodyRange>,
    pub(crate) declarations: Vec<LlvmDeclaration>,
    pub(crate) aliases: Vec<LlvmAlias>,
}

pub(crate) fn scan(path: &Path) -> Result<ModuleIndex> {
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open",
        path: path.to_owned(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut bodies = Vec::new();
    let mut declarations = Vec::new();
    let mut aliases = Vec::new();
    let mut line = Vec::new();
    let mut offset = 0_u64;

    while read_line(&mut reader, &mut line, path)? != 0 {
        let start = offset;
        offset += line.len() as u64;

        if line.starts_with(b"declare ") {
            let symbol = required_global_name(&line, path, "function declaration")?;
            declarations.push(LlvmDeclaration {
                demangled: demangle(&symbol),
                raw_symbol: symbol,
                start,
                end: offset,
            });

            continue;
        }

        if trim_ascii(&line).starts_with(b"@") && is_alias(&line) {
            let symbol = required_global_name(&line, path, "alias")?;
            aliases.push(LlvmAlias {
                demangled: demangle(&symbol),
                target: alias_target(&line, &symbol),
                raw_symbol: symbol,
                start,
                end: offset,
            });

            continue;
        }

        if !line.starts_with(b"define ") {
            continue;
        }

        let symbol = required_global_name(&line, path, "function definition")?;

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

        bodies.push(BodyRange {
            demangled: demangle(&symbol),
            raw_symbol: symbol,
            start,
            end: offset,
        });
    }

    Ok(ModuleIndex {
        bodies,
        declarations,
        aliases,
    })
}

fn required_global_name(line: &[u8], path: &Path, kind: &str) -> Result<String> {
    global_name(line).ok_or_else(|| Error::InvalidLlvm {
        path: path.to_owned(),
        message: format!("{kind} does not contain a global symbol"),
    })
}

fn demangle(symbol: &str) -> String {
    try_demangle(symbol).map_or_else(|_| symbol.to_owned(), |name| format!("{name:#}"))
}

fn is_alias(line: &[u8]) -> bool {
    let content = content_before_comment(line);

    content
        .windows(b" = alias ".len())
        .any(|window| window == b" = alias ")
}

fn alias_target(line: &[u8], alias: &str) -> AliasTarget {
    let Some(alias_start) = find_bytes(line, b" = alias ") else {
        return AliasTarget::Expression;
    };
    let alias_value = &line[alias_start + b" = alias ".len()..];
    let Some(aliasee) = top_level_alias_value(alias_value) else {
        return AliasTarget::Expression;
    };
    let Some(direct_value) = trim_ascii(aliasee).strip_prefix(b"ptr ") else {
        return AliasTarget::Expression;
    };
    let direct_value = trim_ascii(direct_value);
    let symbols = global_names(direct_value);

    match symbols.as_slice() {
        [target]
            if target != alias
                && direct_value.starts_with(b"@")
                && encoded_global_name_length(direct_value) == Some(direct_value.len()) =>
        {
            AliasTarget::Symbol {
                raw_symbol: target.clone(),
            }
        }
        _ => AliasTarget::Expression,
    }
}

fn top_level_alias_value(value: &[u8]) -> Option<&[u8]> {
    let mut depth = 0_u32;

    for (index, byte) in value.iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return Some(&value[index + 1..]),
            _ => {}
        }
    }

    None
}

fn find_bytes(value: &[u8], pattern: &[u8]) -> Option<usize> {
    value
        .windows(pattern.len())
        .position(|window| window == pattern)
}

fn global_names(mut value: &[u8]) -> Vec<String> {
    let mut names = Vec::new();

    while let Some(start) = value.iter().position(|byte| *byte == b'@') {
        value = &value[start..];
        let Some(name) = global_name(value) else {
            break;
        };
        let consumed = encoded_global_name_length(value).unwrap_or(value.len());
        names.push(name);
        value = &value[consumed..];
    }

    names
}

fn encoded_global_name_length(value: &[u8]) -> Option<usize> {
    let start = value.iter().position(|byte| *byte == b'@')? + 1;

    if value.get(start) == Some(&b'"') {
        let mut cursor = start + 1;

        while cursor < value.len() {
            match value[cursor] {
                b'"' => return Some(cursor + 1),
                b'\\' => cursor += 2,
                _ => cursor += 1,
            }
        }

        return None;
    }

    let length = value[start..]
        .iter()
        .position(|byte| !is_identifier_byte(*byte))
        .unwrap_or(value.len() - start);

    Some(start + length)
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
    use std::fs;

    use super::{alias_target, content_before_comment, global_name, is_function_end, scan};
    use crate::AliasTarget;

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

    #[test]
    fn distinguishes_direct_and_expression_aliases() {
        assert_eq!(
            alias_target(b"@alias = alias i32, ptr @target", "alias"),
            AliasTarget::Symbol {
                raw_symbol: "target".to_owned(),
            }
        );
        assert_eq!(
            alias_target(
                b"@alias = alias i8, ptr getelementptr (i8, ptr @target, i64 1)",
                "alias",
            ),
            AliasTarget::Expression
        );
    }

    #[test]
    fn indexes_definitions_declarations_and_aliases() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let path = temporary.path().join("module.ll");
        fs::write(
            &path,
            b"declare void @external(i32)\n\n@alternate = alias void (), ptr @defined\n\ndefine void @defined() {\n  ret void\n}\n",
        )
        .expect("the test can write an LLVM module");

        let index = scan(&path).expect("the LLVM module is valid");

        assert_eq!(index.declarations.len(), 1);
        assert_eq!(index.declarations[0].raw_symbol, "external");
        assert_eq!(index.aliases.len(), 1);
        assert_eq!(
            index.aliases[0].target,
            AliasTarget::Symbol {
                raw_symbol: "defined".to_owned(),
            }
        );
        assert_eq!(index.bodies.len(), 1);
        assert_eq!(index.bodies[0].raw_symbol, "defined");
    }
}
