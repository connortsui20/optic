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

#[cfg(test)]
pub(crate) struct ModuleIndex {
    pub(crate) bodies: Vec<BodyRange>,
    pub(crate) declarations: Vec<LlvmDeclaration>,
    pub(crate) aliases: Vec<LlvmAlias>,
}

pub(crate) enum ModuleRecord {
    Body(BodyRange),

    Declaration(LlvmDeclaration),

    Alias(LlvmAlias),
}

#[cfg(test)]
pub(crate) fn scan(path: &Path) -> Result<ModuleIndex> {
    let mut index = ModuleIndex {
        bodies: Vec::new(),
        declarations: Vec::new(),
        aliases: Vec::new(),
    };
    scan_with(path, |record| match record {
        ModuleRecord::Body(body) => index.bodies.push(body),
        ModuleRecord::Declaration(declaration) => index.declarations.push(declaration),
        ModuleRecord::Alias(alias) => index.aliases.push(alias),
    })?;

    Ok(index)
}

pub(crate) fn scan_with(path: &Path, mut on_record: impl FnMut(ModuleRecord)) -> Result<()> {
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open",
        path: path.to_owned(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut line = ModuleLine::new();
    let mut offset = 0_u64;

    loop {
        line.clear();
        let line_bytes = read_line_with(&mut reader, path, |bytes| line.push(bytes))?;
        if line_bytes == 0 {
            break;
        }

        let start = offset;
        offset += line_bytes;
        let Some(line) = line.record() else {
            continue;
        };

        if line.starts_with(b"declare ") {
            let symbol = required_global_name(line, path, "function declaration")?;
            on_record(ModuleRecord::Declaration(LlvmDeclaration {
                demangled: demangle(&symbol),
                raw_symbol: symbol,
                start,
                end: offset,
            }));

            continue;
        }

        if line.starts_with(b"@") {
            let symbol = required_global_name(line, path, "alias")?;
            on_record(ModuleRecord::Alias(LlvmAlias {
                demangled: demangle(&symbol),
                target: alias_target(line, &symbol),
                raw_symbol: symbol,
                start,
                end: offset,
            }));

            continue;
        }

        let symbol = required_global_name(line, path, "function definition")?;
        let mut function_end = FunctionEnd::new();

        loop {
            function_end.clear();
            let line_bytes = read_line_with(&mut reader, path, |bytes| function_end.push(bytes))?;
            if line_bytes == 0 {
                return Err(Error::InvalidLlvm {
                    path: path.to_owned(),
                    message: "function reached the end of the file without a closing brace"
                        .to_owned(),
                });
            }
            offset += line_bytes;

            if function_end.is_end() {
                break;
            }
        }

        on_record(ModuleRecord::Body(BodyRange {
            demangled: demangle(&symbol),
            raw_symbol: symbol,
            start,
            end: offset,
        }));
    }

    Ok(())
}

/// Buffers a line only after its prefix identifies an indexed LLVM record.
///
/// Large globals, metadata, and function-body lines transition to `Ignore` and discard all
/// subsequent chunks. Record headers remain available to the existing symbol parsers.
struct ModuleLine {
    state: ModuleLineState,
    bytes: Vec<u8>,
}

impl ModuleLine {
    fn new() -> Self {
        Self {
            state: ModuleLineState::Start,
            bytes: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.state = ModuleLineState::Start;
        self.bytes.clear();
    }

    fn push(&mut self, bytes: &[u8]) {
        if matches!(self.state, ModuleLineState::Ignore) {
            return;
        }
        if matches!(self.state, ModuleLineState::Record) {
            self.bytes.extend_from_slice(bytes);
            return;
        }

        for (index, byte) in bytes.iter().enumerate() {
            self.push_byte(*byte);

            if matches!(self.state, ModuleLineState::Ignore) {
                return;
            }
            if matches!(self.state, ModuleLineState::Record) {
                self.bytes.extend_from_slice(&bytes[index + 1..]);
                return;
            }
        }
    }

    fn push_byte(&mut self, byte: u8) {
        match &mut self.state {
            ModuleLineState::Start => {
                if byte.is_ascii_whitespace() {
                    self.state = ModuleLineState::LeadingWhitespace;
                } else if byte == b'd' {
                    self.bytes.push(byte);
                    self.state = ModuleLineState::Keyword;
                } else if byte == b'@' {
                    self.bytes.push(byte);
                    self.state = ModuleLineState::Alias(AliasPrefix::new());
                } else {
                    self.state = ModuleLineState::Ignore;
                }
            }
            ModuleLineState::LeadingWhitespace => {
                if byte == b'@' {
                    self.bytes.push(byte);
                    self.state = ModuleLineState::Alias(AliasPrefix::new());
                } else if !byte.is_ascii_whitespace() {
                    self.state = ModuleLineState::Ignore;
                }
            }
            ModuleLineState::Keyword => {
                self.bytes.push(byte);

                const KEYWORDS: [&[u8]; 2] = [b"declare ", b"define "];
                if KEYWORDS.iter().any(|keyword| *keyword == self.bytes) {
                    self.state = ModuleLineState::Record;
                } else if !KEYWORDS
                    .iter()
                    .any(|keyword| keyword.starts_with(&self.bytes))
                {
                    self.bytes.clear();
                    self.state = ModuleLineState::Ignore;
                }
            }
            ModuleLineState::Alias(prefix) => {
                self.bytes.push(byte);

                match prefix.push(byte) {
                    AliasPrefixResult::Continue => {}
                    AliasPrefixResult::Record => self.state = ModuleLineState::Record,
                    AliasPrefixResult::Ignore => {
                        self.bytes.clear();
                        self.state = ModuleLineState::Ignore;
                    }
                }
            }
            ModuleLineState::Record | ModuleLineState::Ignore => {
                unreachable!("completed line states are handled before byte classification")
            }
        }
    }

    fn record(&self) -> Option<&[u8]> {
        matches!(self.state, ModuleLineState::Record).then_some(self.bytes.as_slice())
    }
}

enum ModuleLineState {
    Start,
    LeadingWhitespace,
    Keyword,
    Alias(AliasPrefix),
    Record,
    Ignore,
}

struct AliasPrefix {
    state: AliasPrefixState,
}

impl AliasPrefix {
    fn new() -> Self {
        Self {
            state: AliasPrefixState::NameStart,
        }
    }

    fn push(&mut self, byte: u8) -> AliasPrefixResult {
        match self.state {
            AliasPrefixState::NameStart if byte == b'"' => {
                self.state = AliasPrefixState::QuotedName;
            }
            AliasPrefixState::NameStart if is_identifier_byte(byte) => {
                self.state = AliasPrefixState::UnquotedName;
            }
            AliasPrefixState::NameStart => return self.push_pattern(byte, 0),
            AliasPrefixState::UnquotedName if !is_identifier_byte(byte) => {
                return self.push_pattern(byte, 0);
            }
            AliasPrefixState::UnquotedName => {}
            AliasPrefixState::QuotedName if byte == b'\\' => {
                self.state = AliasPrefixState::QuotedEscape;
            }
            AliasPrefixState::QuotedName if byte == b'"' => {
                self.state = AliasPrefixState::Pattern(0);
            }
            AliasPrefixState::QuotedName => {}
            AliasPrefixState::QuotedEscape => self.state = AliasPrefixState::QuotedName,
            AliasPrefixState::Pattern(matched) => return self.push_pattern(byte, matched),
        }

        AliasPrefixResult::Continue
    }

    fn push_pattern(&mut self, byte: u8, matched: usize) -> AliasPrefixResult {
        const ALIAS_PATTERN: &[u8] = b" = alias ";

        if ALIAS_PATTERN.get(matched) != Some(&byte) {
            return AliasPrefixResult::Ignore;
        }

        let matched = matched + 1;
        if matched == ALIAS_PATTERN.len() {
            AliasPrefixResult::Record
        } else {
            self.state = AliasPrefixState::Pattern(matched);
            AliasPrefixResult::Continue
        }
    }
}

enum AliasPrefixState {
    NameStart,
    UnquotedName,
    QuotedName,
    QuotedEscape,
    Pattern(usize),
}

enum AliasPrefixResult {
    Continue,
    Record,
    Ignore,
}

/// Recognizes a closing-brace line without retaining function-body bytes.
struct FunctionEnd {
    state: FunctionEndState,
}

impl FunctionEnd {
    fn new() -> Self {
        Self {
            state: FunctionEndState::LeadingWhitespace,
        }
    }

    fn clear(&mut self) {
        self.state = FunctionEndState::LeadingWhitespace;
    }

    fn push(&mut self, bytes: &[u8]) {
        for byte in bytes {
            match self.state {
                FunctionEndState::LeadingWhitespace if byte.is_ascii_whitespace() => {}
                FunctionEndState::LeadingWhitespace if *byte == b'}' => {
                    self.state = FunctionEndState::AfterBrace;
                }
                FunctionEndState::LeadingWhitespace => {
                    self.state = FunctionEndState::NotEnd;
                }
                FunctionEndState::AfterBrace if byte.is_ascii_whitespace() => {}
                FunctionEndState::AfterBrace if *byte == b';' => {
                    self.state = FunctionEndState::Comment;
                }
                FunctionEndState::AfterBrace => self.state = FunctionEndState::NotEnd,
                FunctionEndState::Comment | FunctionEndState::NotEnd => {}
            }

            if matches!(
                self.state,
                FunctionEndState::Comment | FunctionEndState::NotEnd
            ) {
                return;
            }
        }
    }

    fn is_end(&self) -> bool {
        matches!(
            self.state,
            FunctionEndState::AfterBrace | FunctionEndState::Comment
        )
    }
}

enum FunctionEndState {
    LeadingWhitespace,
    AfterBrace,
    Comment,
    NotEnd,
}

/// Streams one line in reader-sized chunks and returns its exact byte length.
fn read_line_with(
    reader: &mut impl BufRead,
    path: &Path,
    mut on_bytes: impl FnMut(&[u8]),
) -> Result<u64> {
    let mut total_bytes = 0_u64;

    loop {
        let (bytes_read, finished) = {
            let available = reader.fill_buf().map_err(|source| Error::Filesystem {
                operation: "read",
                path: path.to_owned(),
                source,
            })?;
            if available.is_empty() {
                return Ok(total_bytes);
            }

            let bytes_read = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            let finished = available[bytes_read - 1] == b'\n';
            on_bytes(&available[..bytes_read]);

            (bytes_read, finished)
        };

        reader.consume(bytes_read);
        total_bytes = total_bytes.saturating_add(bytes_read as u64);
        if finished {
            return Ok(total_bytes);
        }
    }
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

    use super::{FunctionEnd, ModuleLine, alias_target, global_name, scan};
    use crate::AliasTarget;

    fn is_function_end(line: &[u8]) -> bool {
        let mut detector = FunctionEnd::new();
        detector.push(line);

        detector.is_end()
    }

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
    fn does_not_buffer_a_large_irrelevant_line() {
        let mut line = ModuleLine::new();
        line.push(b"@bytes = private constant [1048576 x i8] c\"");
        line.push(&vec![b'x'; 1024 * 1024]);
        line.push(b"\"\n");

        assert!(line.record().is_none());
        assert!(line.bytes.is_empty());
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
