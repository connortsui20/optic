//! Selects top-level definitions and constructs validated records in input order.
//!
//! The disassembler emits unrelated constructs on separate lines. Only function and global prefixes
//! enter the header tokenizer, so unrelated initializers and metadata need no token storage.

use std::io::{self, BufRead};

use optic_records::{ByteRange, LlvmDefinitionKind, LlvmDefinitionRecord};

use super::header;
use super::stream::{Stream, invalid};

/// Indexes exact decoded symbols and unchanged byte ranges from LLVM disassembler output.
///
/// Function ranges start at `define` and end after the closing body brace. Alias and ifunc ranges
/// start at `@` and include the terminating newline, when present. Empty input produces no records.
/// Headers have a 1 MiB input budget. Body content and unrelated lines use a fixed 8 KiB buffer.
///
/// Returns `InvalidData` with a byte offset for malformed boundaries, unsupported header syntax,
/// non-UTF-8 symbols, or a header over budget. Reader errors retain their original kind.
pub(crate) fn index(reader: impl BufRead) -> io::Result<Vec<LlvmDefinitionRecord>> {
    scan(&mut Stream::new(reader))
}

fn scan(reader: &mut Stream<impl BufRead>) -> io::Result<Vec<LlvmDefinitionRecord>> {
    let mut definitions = Vec::new();

    while let Some(byte) = reader.peek()? {
        if byte.is_ascii_whitespace() {
            reader.take()?;
            continue;
        }

        let start = reader.offset();

        if byte == b'@' {
            reader.begin_header(start);
            let symbol = reader.required_token()?;
            reader.expect(b"=")?;

            if let Some(kind) = global(reader)? {
                let symbol = symbol.symbol()?;
                let range = ByteRange::new(start, reader.offset() - start)
                    .map_err(|error| invalid(start, error))?;
                definitions.push(
                    LlvmDefinitionRecord::new(symbol, range, kind)
                        .map_err(|error| invalid(start, error))?,
                );
            }

            reader.end_header();
            continue;
        }

        // AssemblyWriter emits define/declare at the start of a line. Probe only six bytes before
        // discarding an unrelated line, even when that line contains a very long first token.
        // See https://github.com/llvm/llvm-project/blob/llvmorg-22.1.0/llvm/lib/IR/AsmWriter.cpp.
        let mut prefix = [0; 6];

        if byte == b'd' {
            for next in &mut prefix {
                let Some(byte) = reader.take()? else { break };
                *next = byte;

                if byte == b'\n' {
                    break;
                }
            }
        }

        if prefix == *b"define" && reader.peek()?.is_none_or(|byte| byte.is_ascii_whitespace()) {
            reader.begin_header(start);
            let symbol = header::function(reader)?;
            reader.end_header();
            reader.body()?;

            let range = ByteRange::new(start, reader.offset() - start)
                .map_err(|error| invalid(start, error))?;
            definitions.push(
                LlvmDefinitionRecord::new(symbol, range, LlvmDefinitionKind::Function)
                    .map_err(|error| invalid(start, error))?,
            );
        } else if !prefix.contains(&b'\n') {
            reader.skip_line()?;
        }
    }

    Ok(definitions)
}

fn global(reader: &mut Stream<impl BufRead>) -> io::Result<Option<LlvmDefinitionKind>> {
    // These modifiers precede global/constant, alias, or ifunc. Stop before an unrelated initializer.
    // See https://llvm.org/docs/LangRef.html#global-variables and #aliases.
    loop {
        let token = reader.required_token()?;

        match token.bytes.as_slice() {
            b"global" | b"constant" => {
                reader.end_header();
                reader.skip_line()?;
                return Ok(None);
            }
            b"alias" => {
                let target = header::alias_target(reader)?;
                return Ok(Some(match target {
                    Some(target) => LlvmDefinitionKind::DirectAlias { target },
                    None => LlvmDefinitionKind::ExpressionAlias,
                }));
            }
            b"ifunc" => {
                header::alias_target(reader)?;
                return Ok(Some(LlvmDefinitionKind::Ifunc));
            }
            b"private"
            | b"internal"
            | b"available_externally"
            | b"linkonce"
            | b"weak"
            | b"common"
            | b"appending"
            | b"extern_weak"
            | b"linkonce_odr"
            | b"weak_odr"
            | b"external"
            | b"dso_local"
            | b"dso_preemptable"
            | b"default"
            | b"hidden"
            | b"protected"
            | b"dllimport"
            | b"dllexport"
            | b"unnamed_addr"
            | b"local_unnamed_addr"
            | b"externally_initialized"
            | b"thread_local" => {}
            b"addrspace" => {
                reader.expect(b"(")?;
                reader.group(b'(')?;
            }
            b"(" => reader.group(b'(')?,
            _ => {
                return Err(invalid(
                    token.offset,
                    "Expected an LLVM global, alias, or ifunc",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor, ErrorKind, Read};

    use super::super::stream::HEADER_LIMIT;
    use super::*;

    #[track_caller]
    fn check(input: &str, expected: &[(&str, LlvmDefinitionKind, &str)]) {
        for capacity in [1, 2, 7, 8192] {
            let actual =
                super::super::index(BufReader::with_capacity(capacity, input.as_bytes())).unwrap();
            assert_eq!(actual.len(), expected.len());

            for (definition, (symbol, kind, excerpt)) in actual.iter().zip(expected) {
                assert_eq!(definition.raw_symbol(), *symbol);
                assert_eq!(definition.kind(), kind);
                let range = definition.range();
                assert_eq!(
                    &input[range.start() as usize..range.end() as usize],
                    *excerpt
                );
            }
        }
    }

    #[test]
    fn exact_definition_ranges_and_kinds() {
        let function = "define void @f() {\n  ret void\n}";
        let alias = "@a = weak hidden local_unnamed_addr alias void (), ptr @f\n";
        let expression = "@e = alias i8, getelementptr (i8, ptr @g, i64 1)\n";
        let ifunc = "@i = ifunc void (), ptr @resolver\n";
        let input = format!(
            "; define @fake\ndeclare void @decl()\n@g = global i8 0\n{alias}{expression}{ifunc}\n{function}\n!0 = !{{ptr @f}}\n"
        );

        check(
            &input,
            &[
                (
                    "a",
                    LlvmDefinitionKind::DirectAlias { target: "f".into() },
                    alias,
                ), // Direct alias.
                ("e", LlvmDefinitionKind::ExpressionAlias, expression), // Expression alias.
                ("i", LlvmDefinitionKind::Ifunc, ifunc),                // Unsupported resolver.
                ("f", LlvmDefinitionKind::Function, function),          // Whole body.
            ],
        );
    }

    #[test]
    fn multiline_quoted_alias_and_function() {
        let alias = "@\"alias\\20name\"\n = linkonce_odr\n dso_local hidden\n alias void (),\n ptr @\"raw\\22\\\\name\"\n";
        let function = "define void\n @\"raw\\22\\\\name\"(\n) { ret void }";
        let input = format!("{alias}{function}");

        check(
            &input,
            &[
                (
                    "alias name",
                    LlvmDefinitionKind::DirectAlias {
                        target: "raw\"\\name".into(),
                    },
                    alias,
                ), // Quoted alias.
                ("raw\"\\name", LlvmDefinitionKind::Function, function), // Quoted definition.
            ],
        );
    }

    #[test]
    fn disassembler_fixture_matches_line_reference() {
        let input = include_str!("fixtures/definitions.ll");
        let expected = [
            (
                "direct",
                LlvmDefinitionKind::DirectAlias {
                    target: "f\"\\\u{1}é".into(),
                },
            ), // Direct alias.
            ("expression", LlvmDefinitionKind::ExpressionAlias), // Untyped expression.
            ("indirect", LlvmDefinitionKind::Ifunc),             // Runtime resolver.
            ("f\"\\\u{1}é", LlvmDefinitionKind::Function),       // Escaped definition.
            ("resolver", LlvmDefinitionKind::Function),          // Ordinary definition.
        ];
        let mut excerpts = Vec::new();
        let mut offset = 0;

        // This reference uses complete lines and the fixture's canonical function closing lines.
        // The production scanner must obtain the same ranges without retaining those lines.
        for line in input.split_inclusive('\n') {
            if line.starts_with("define ") {
                let length = input[offset..].find("\n}").unwrap() + 2;
                excerpts.push(&input[offset..offset + length]);
            } else if line.starts_with('@') && !line.starts_with("@data ") {
                excerpts.push(line);
            }

            offset += line.len();
        }

        let expected: Vec<_> = expected
            .into_iter()
            .zip(excerpts)
            .map(|((symbol, kind), excerpt)| (symbol, kind, excerpt))
            .collect();
        check(input, &expected);
    }

    #[test]
    fn unrelated_input_has_no_definitions() {
        for input in [
            "",                                                        // Empty module.
            "\n ; comment\r\n",                                        // Whitespace.
            "declare void @f()\n",                                     // Declaration.
            "define_like_long_token\n",                                // Keyword prefix.
            "d\ndefine_like\n",                                        // Short unrelated line.
            "@g = thread_local(localexec) addrspace(1) global i8 0\n", // Modified global.
            "module asm \"define void @fake() {}\"\n",                 // Text inside assembly.
        ] {
            check(input, &[]);
        }
    }

    #[test]
    fn range_endings_preserve_input_bytes() {
        for ending in ["\n", "\r\n", ""] {
            let alias = format!("@a = alias void (), ptr @f{ending}");
            check(
                &alias,
                &[(
                    "a",
                    LlvmDefinitionKind::DirectAlias { target: "f".into() },
                    &alias,
                )],
            );

            let function = format!("define void @f() {{{ending} ret void{ending}}}");
            let input = format!("  {function}{ending}");
            check(&input, &[("f", LlvmDefinitionKind::Function, &function)]);
        }
    }

    #[test]
    fn large_unrelated_line_and_body_have_bounded_buffers() {
        let length = HEADER_LIMIT * 8;
        let input = Cursor::new(b"@g = private constant [8388608 x i8] c\"")
            .chain(std::io::repeat(b'x').take(length))
            .chain(Cursor::new(b"\"\ndefine void @f() {\n;"))
            .chain(std::io::repeat(b'x').take(length))
            .chain(Cursor::new(b"\n ret void\n}"));
        let mut stream = Stream::new(BufReader::new(input));
        let definitions = scan(&mut stream).unwrap();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].raw_symbol(), "f");
        assert!(definitions[0].range().start() > length);
        assert!(definitions[0].range().length() > length);
        let (chunk, token, group) = stream.buffer_peaks();
        assert_eq!(chunk, 8192);
        assert!(token <= 16);
        assert!(group <= 8);
    }

    #[test]
    fn complete_header_budget_boundaries() {
        for prefix in ["define void @f() ", "@a = alias void (), ptr "] {
            let suffix = if prefix.starts_with("define") {
                "{"
            } else {
                "@f\n"
            };
            for length in [HEADER_LIMIT - 1, HEADER_LIMIT, HEADER_LIMIT + 1] {
                let padding = length - (prefix.len() + suffix.len()) as u64;
                let input = Cursor::new(prefix.as_bytes())
                    .chain(std::io::repeat(b' ').take(padding))
                    .chain(Cursor::new(suffix.as_bytes()))
                    .chain(Cursor::new(if prefix.starts_with("define") {
                        &b"ret void}"[..]
                    } else {
                        b""
                    }));
                let result = index(BufReader::new(input));

                assert_eq!(result.is_ok(), length <= HEADER_LIMIT);
                if let Err(error) = result {
                    assert_eq!(error.kind(), ErrorKind::InvalidData);
                    assert!(error.to_string().contains("1 MiB"));
                    assert!(error.to_string().contains("at byte 1048576"));
                }
            }
        }
    }

    #[test]
    fn malformed_definitions_have_offset_context() {
        for input in [
            "define",                        // Truncated keyword at EOF.
            "define void @f() {",            // Truncated body.
            "@a = alias void (), ptr",       // Truncated alias.
            "@i = ifunc void (), ptr",       // Truncated ifunc.
            "@a = alias void (), ptr @\"\"", // Invalid record target.
            "@\"\" = alias void (), ptr @f", // Invalid record symbol.
            "@a alias void (), ptr @f",      // Missing assignment.
        ] {
            let error = index(input.as_bytes()).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData);
            assert!(error.to_string().contains("at byte"));
        }
    }

    #[test]
    fn reader_failures_preserve_their_kind() {
        struct Failure;

        impl Read for Failure {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "fixture reader failed",
                ))
            }
        }

        for prefix in ["", "define void @f() {", "@a = alias void (), ptr "] {
            let input = Cursor::new(prefix.as_bytes()).chain(Failure);
            let error = index(BufReader::new(input)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::BrokenPipe);
            assert_eq!(error.to_string(), "fixture reader failed");
        }
    }
}
