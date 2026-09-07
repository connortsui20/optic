//! Reads function headers and alias operands from LLVM tokens.
//!
//! Delimiter checks establish definition boundaries. The scanner does not verify LLVM types or
//! instruction semantics, which the disassembler already validated.

use std::io::{self, BufRead};

use super::stream::{Stream, Token, invalid, matching_close};

/// Reads a function name and consumes the opening body brace.
pub(super) fn read_function(reader: &mut Stream<impl BufRead>) -> io::Result<String> {
    // The function name precedes its argument list. Result types and attributes can contain groups.
    // Prefix, prologue, and personality constants follow the argument list before the body.
    // See https://llvm.org/docs/LangRef.html#functions.
    let symbol = loop {
        let token = reader.read_required_token()?;

        if token.bytes.starts_with(b"@") {
            break token.decode_symbol()?;
        }

        if matches!(token.bytes.as_slice(), b"define" | b"declare" | b"}") {
            return Err(invalid(token.offset, "Expected an LLVM function name"));
        }

        consume_group(reader, &token)?;
    };

    reader.expect(b"(")?;
    reader.consume_group(b'(')?;

    loop {
        let token = reader.read_required_token()?;

        match token.bytes.as_slice() {
            b"{" => return Ok(symbol),
            b"prefix" | b"prologue" | b"personality" => consume_typed_constant(reader)?,
            b"!" => {
                let metadata = reader.read_required_token()?;
                consume_group(reader, &metadata)?;
            }
            b"define" | b"declare" | b"}" => {
                return Err(invalid(token.offset, "Expected an LLVM function body"));
            }
            _ => consume_group(reader, &token)?,
        }
    }
}

/// Reads the alias type and operand, preserving only a direct global target.
pub(super) fn read_alias_target(reader: &mut Stream<impl BufRead>) -> io::Result<Option<String>> {
    // An alias has a type, a comma, and a typed global value or constant expression.
    // A symbol inside a constant expression is not a direct alias target.
    // See https://llvm.org/docs/LangRef.html#aliases and https://llvm.org/docs/LangRef.html#ifuncs.
    let first = reader.read_required_token()?;

    if first.bytes == b"," {
        return Err(invalid(first.offset, "Expected an LLVM alias type"));
    }

    consume_group(reader, &first)?;

    loop {
        let token = reader.read_required_token()?;

        if token.bytes == b"," {
            break;
        }

        if matches!(token.bytes.as_slice(), b"=" | b"define" | b"declare")
            || token.bytes.starts_with(b"@")
        {
            return Err(invalid(
                token.offset,
                "Expected a comma after the LLVM alias type",
            ));
        }

        consume_group(reader, &token)?;
    }

    let mut operand = reader.read_required_token()?;
    let typed = operand.bytes == b"ptr";

    // AssemblyWriter::printAlias and printIFunc omit the type for constant expressions.
    // See https://github.com/llvm/llvm-project/blob/llvmorg-22.1.0/llvm/lib/IR/AsmWriter.cpp.
    if typed {
        operand = reader.read_required_token()?;

        if operand.bytes == b"addrspace" {
            reader.expect(b"(")?;
            reader.consume_group(b'(')?;
            operand = reader.read_required_token()?;
        }
    }

    let target = if typed && operand.bytes.starts_with(b"@") {
        Some(operand.decode_symbol()?)
    } else {
        if operand.bytes.starts_with(b"@") {
            return Err(invalid(
                operand.offset,
                "Expected an LLVM pointer type before the target",
            ));
        }

        consume_constant(reader, operand)?;
        None
    };

    while let Some(token) = reader.read_token(true)? {
        if token.bytes != b"," {
            return Err(invalid(
                token.offset,
                "Expected an LLVM alias suffix, got another operand",
            ));
        }

        let suffix = reader.read_required_token()?;

        if suffix.bytes.starts_with(b"!") {
            let metadata = reader.read_required_token()?;
            consume_group(reader, &metadata)?;

            continue;
        }

        if suffix.bytes != b"partition" {
            return Err(invalid(
                suffix.offset,
                "Expected an LLVM partition or metadata suffix",
            ));
        }

        let name = reader.read_required_token()?;

        if !name.bytes.starts_with(b"\"") {
            return Err(invalid(name.offset, "Expected an LLVM partition string"));
        }
    }

    Ok(target)
}

fn consume_typed_constant(reader: &mut Stream<impl BufRead>) -> io::Result<()> {
    let ty = reader.read_required_token()?;
    consume_group(reader, &ty)?;
    let mut value = reader.read_required_token()?;

    if ty.bytes == b"ptr" && value.bytes == b"addrspace" {
        reader.expect(b"(")?;
        reader.consume_group(b'(')?;
        value = reader.read_required_token()?;
    }

    consume_constant(reader, value)
}

fn consume_constant(reader: &mut Stream<impl BufRead>, token: Token) -> io::Result<()> {
    if token.bytes.len() == 1 && matching_close(token.bytes[0]).is_some() {
        return reader.consume_group(token.bytes[0]);
    }

    if token.bytes.starts_with(b"@") {
        token.decode_symbol()?;

        return Ok(());
    }

    if matches!(token.bytes.as_slice(), b"dso_local_equivalent" | b"no_cfi") {
        reader.read_required_token()?.decode_symbol()?;

        return Ok(());
    }

    if token.bytes == b"c" {
        let string = reader.read_required_token()?;

        if !string.bytes.starts_with(b"\"") {
            return Err(invalid(string.offset, "Expected an LLVM byte string"));
        }

        return Ok(());
    }

    if matches!(
        token.bytes.as_slice(),
        b"null" | b"undef" | b"poison" | b"zeroinitializer" | b"true" | b"false"
    ) || token.bytes.first().is_some_and(u8::is_ascii_digit)
        || token.bytes.starts_with(b"-")
    {
        return Ok(());
    }

    // Constant expressions use an opcode and a parenthesized operand list. GEP can add flags.
    // See https://llvm.org/docs/LangRef.html#constant-expressions.
    if !matches!(
        token.bytes.as_slice(),
        b"getelementptr"
            | b"bitcast"
            | b"addrspacecast"
            | b"inttoptr"
            | b"ptrtoint"
            | b"trunc"
            | b"blockaddress"
            | b"ptrauth"
            | b"splat"
    ) {
        return Err(invalid(
            token.offset,
            "Expected a supported LLVM constant form",
        ));
    }

    let mut next = reader.read_required_token()?;

    if token.bytes == b"getelementptr" {
        while matches!(next.bytes.as_slice(), b"inbounds" | b"nuw" | b"nusw") {
            next = reader.read_required_token()?;
        }
    }

    if next.bytes != b"(" {
        return Err(invalid(
            next.offset,
            "Expected LLVM constant operands in parentheses",
        ));
    }

    reader.consume_group(b'(')
}

fn consume_group(reader: &mut Stream<impl BufRead>, token: &Token) -> io::Result<()> {
    if token.bytes.len() != 1 {
        return Ok(());
    }

    let byte = token.bytes[0];

    if matching_close(byte).is_some() {
        reader.consume_group(byte)?;
    } else if matches!(byte, b')' | b']' | b'}' | b'>') {
        return Err(invalid(token.offset, "Expected an opening LLVM delimiter"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::super::tests::READ_LIMITS;
    use super::super::tests::ShortReader;
    use super::*;

    #[track_caller]
    fn parse_function(input: &str, max_read: usize) -> io::Result<String> {
        let mut stream = Stream::new(ShortReader::new(input.as_bytes(), max_read));
        stream.begin_header(0);

        let name = read_function(&mut stream)?;
        stream.end_header();
        stream.skip_body()?;

        Ok(name)
    }

    #[track_caller]
    fn parse_alias(input: &str, max_read: usize) -> io::Result<Option<String>> {
        let mut stream = Stream::new(ShortReader::new(input.as_bytes(), max_read));
        stream.begin_header(0);

        read_alias_target(&mut stream)
    }

    #[test]
    fn function_header_dimensions() {
        for header in [
            "void @f() { ret void }", // Empty arguments.
            "{i8, ptr} @f(ptr byval({i8, ptr}) %x) { \
             ret {i8, ptr} zeroinitializer }", // The types are aggregates.
            "dso_local noundef i32 @f(\n i32 %x,\n ptr %y\n) #0\n \
             section \"{@fake}\" { ret i32 0 }", // The attributes span lines.
            "void @f() prefix {i32, i8} {i32 1, i8 2} prologue [2 x i8] c\"ab\" \
             personality ptr @p { ret void }", // Constants occur in the header.
            "void @f() prefix ptr getelementptr (i8, ptr @g, i64 1) !dbg !0 \
             { ret void }", // The constant is an expression.
        ] {
            for max_read in READ_LIMITS {
                assert_eq!(parse_function(header, max_read).unwrap(), "f");
            }
        }
    }

    #[test]
    fn alias_operand_dimensions() {
        let cases = [
            ("void (), ptr @target\n", Some("target")), // Ordinary direct target.
            (
                "{i8, i8},\n ptr addrspace(1)\n @\"raw\\20target\"\n",
                Some("raw target"),
            ), // Multiline quoted target.
            ("void (), ptr @target, partition \"part\"\n", Some("target")), // Partition suffix.
            ("void (), ptr @target, !dbg !0\n", Some("target")), // IFunc metadata.
            (
                "i8, ptr getelementptr inbounds (i8, ptr @target, i64 1)\n",
                None,
            ), // Expression alias.
            ("i8, ptr inttoptr (i64 4 to ptr)", None),  // No global target.
            (
                "i8, getelementptr inbounds (i8, ptr @target, i64 1)\n",
                None,
            ), // Disassembler expression spelling.
            ("void (), ptr no_cfi @target\n", None),    // Wrapped global target.
        ];

        for (input, expected) in cases {
            for max_read in READ_LIMITS {
                assert_eq!(parse_alias(input, max_read).unwrap().as_deref(), expected);
            }
        }
    }

    #[test]
    fn malformed_headers_fail() {
        for input in [
            "void @f",                 // No arguments.
            "void @f(]",               // Mismatched arguments.
            "void @f()",               // No body.
            "void @f() prefix ptr @x", // No body after a constant.
            "void @f() { ret void",    // Truncated body.
        ] {
            assert_eq!(
                parse_function(input, 1).unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        for input in [
            ", ptr @x",                               // No alias type.
            "void () ptr @x",                         // No comma.
            "void (), ptr",                           // No operand.
            "void (), ptr @x @y",                     // Extra operand.
            "void (), ptr getelementptr (i8, ptr @x", // Truncated expression.
            "void (), ptr nonsense @x",               // Unknown constant form.
            "void (), ptr @x, partition 1",           // Invalid partition.
        ] {
            assert_eq!(
                parse_alias(input, 8192).unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }
    }
}
