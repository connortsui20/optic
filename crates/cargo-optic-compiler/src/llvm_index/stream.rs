//! Reads LLVM header tokens and body boundaries with bounded storage.
//!
//! Header budgets count input bytes, including whitespace and comments. Body scans retain no tokens.

use std::io::{self, BufRead, ErrorKind};

// The show contract sets this parser budget. It is not an LLVM language limit.
// See docs/design/show.md on the planning branch, under "Exact indexing".
pub(super) const HEADER_LIMIT: u64 = 1024 * 1024;
const CHUNK_SIZE: usize = 8192;

pub(super) struct Stream<R> {
    reader: R,
    chunk: [u8; CHUNK_SIZE],
    position: usize,
    length: usize,
    offset: u64,
    header_start: Option<u64>,
    #[cfg(test)]
    peak_token_capacity: usize,
    #[cfg(test)]
    peak_group_capacity: usize,
}

pub(super) struct Token {
    /// The complete token, including its prefix and quotes.
    pub(super) bytes: Vec<u8>,
    /// The byte offset of the first token byte.
    pub(super) offset: u64,
}

impl<R: BufRead> Stream<R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            reader,
            chunk: [0; CHUNK_SIZE],
            position: 0,
            length: 0,
            offset: 0,
            header_start: None,
            #[cfg(test)]
            peak_token_capacity: 0,
            #[cfg(test)]
            peak_group_capacity: 0,
        }
    }

    pub(super) fn offset(&self) -> u64 {
        self.offset
    }

    pub(super) fn begin_header(&mut self, start: u64) {
        self.header_start = Some(start);
    }

    pub(super) fn end_header(&mut self) {
        self.header_start = None;
    }

    pub(super) fn peek(&mut self) -> io::Result<Option<u8>> {
        if self.position == self.length {
            loop {
                match self.reader.read(&mut self.chunk) {
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    result => self.length = result?,
                }

                break;
            }

            self.position = 0;
        }

        Ok((self.position < self.length).then(|| self.chunk[self.position]))
    }

    pub(super) fn take(&mut self) -> io::Result<Option<u8>> {
        let Some(byte) = self.peek()? else {
            return Ok(None);
        };

        if let Some(start) = self.header_start
            && self.offset - start == HEADER_LIMIT
        {
            return Err(invalid(self.offset, "LLVM header exceeds the 1 MiB budget"));
        }

        self.offset = self
            .offset
            .checked_add(1)
            .ok_or_else(|| invalid(self.offset, "LLVM byte offset exceeds u64"))?;
        self.position += 1;

        Ok(Some(byte))
    }

    pub(super) fn skip_line(&mut self) -> io::Result<()> {
        while let Some(byte) = self.take()? {
            if byte == b'\n' {
                break;
            }
        }

        Ok(())
    }

    /// Reads one token, optionally treating a newline as the end of a header.
    pub(super) fn token(&mut self, stop_at_newline: bool) -> io::Result<Option<Token>> {
        assert!(
            self.header_start.is_some(),
            "header tokens require an active byte budget"
        );

        while let Some(byte) = self.peek()? {
            if byte == b';' {
                self.skip_line()?;

                if stop_at_newline {
                    return Ok(None);
                }
            } else if byte.is_ascii_whitespace() {
                self.take()?;

                if stop_at_newline && byte == b'\n' {
                    return Ok(None);
                }
            } else {
                break;
            }
        }

        let offset = self.offset;
        let Some(first) = self.take()? else {
            return Ok(None);
        };
        let mut bytes = vec![first];

        // Retain the result until the test seam observes allocations from rejected tokens too.
        let result: io::Result<()> = (|| {
            if first == b'"' {
                self.quoted(&mut bytes)?;
            } else if matches!(first, b'@' | b'%' | b'$') && self.peek()? == Some(b'"') {
                bytes.push(self.take()?.expect("peek established the opening quote"));
                self.quoted(&mut bytes)?;
            } else if is_word(first) || matches!(first, b'@' | b'%' | b'$' | b'!') {
                while self.peek()?.is_some_and(is_word) {
                    bytes.push(self.take()?.expect("peek established the next token byte"));
                }
            }

            Ok(())
        })();

        #[cfg(test)]
        {
            self.peak_token_capacity = self.peak_token_capacity.max(bytes.capacity());
        }

        result?;

        Ok(Some(Token { bytes, offset }))
    }

    pub(super) fn required_token(&mut self) -> io::Result<Token> {
        self.token(false)?
            .ok_or_else(|| invalid(self.offset, "Expected an LLVM header token, got EOF"))
    }

    pub(super) fn expect(&mut self, expected: &[u8]) -> io::Result<()> {
        let token = self.required_token()?;

        if token.bytes != expected {
            return Err(invalid(
                token.offset,
                format!("Expected LLVM token {}", String::from_utf8_lossy(expected)),
            ));
        }

        Ok(())
    }

    fn quoted(&mut self, bytes: &mut Vec<u8>) -> io::Result<()> {
        loop {
            let byte = self
                .take()?
                .ok_or_else(|| invalid(self.offset, "Expected a closing LLVM quote, got EOF"))?;
            bytes.push(byte);

            match byte {
                b'"' => return Ok(()),
                b'\n' | b'\r' => {
                    return Err(invalid(self.offset - 1, "Expected an escaped LLVM newline"));
                }
                b'\\' => {
                    if self.peek()? == Some(b'\\') {
                        bytes.push(
                            self.take()?
                                .expect("peek established the escaped backslash"),
                        );
                        continue;
                    }

                    for _ in 0..2 {
                        let hex = self.take()?.ok_or_else(|| {
                            invalid(self.offset, "Expected two LLVM escape digits, got EOF")
                        })?;

                        if !hex.is_ascii_hexdigit() {
                            return Err(invalid(self.offset - 1, "Expected an LLVM hex escape"));
                        }

                        bytes.push(hex);
                    }
                }
                _ => {}
            }
        }
    }

    /// Consumes a balanced header group without recursion.
    pub(super) fn group(&mut self, opening: u8) -> io::Result<()> {
        let mut closing =
            vec![matching_close(opening).expect("group callers supply an opening delimiter")];

        while let Some(&expected) = closing.last() {
            #[cfg(test)]
            {
                self.peak_group_capacity = self.peak_group_capacity.max(closing.capacity());
            }

            let token = self.required_token()?;

            if token.bytes.len() != 1 {
                continue;
            }

            let byte = token.bytes[0];

            if let Some(close) = matching_close(byte) {
                closing.push(close);
            } else if matches!(byte, b')' | b']' | b'}' | b'>') {
                if byte != expected {
                    return Err(invalid(token.offset, "Expected a matching LLVM delimiter"));
                }

                closing.pop();
            }
        }

        Ok(())
    }

    /// Reports independent allocation peaks, excluding the returned definition records.
    #[cfg(test)]
    pub(super) fn buffer_peaks(&self) -> (usize, usize, usize) {
        (
            CHUNK_SIZE,
            self.peak_token_capacity,
            self.peak_group_capacity,
        )
    }

    /// Consumes the body after its opening brace and returns just after the closing brace.
    pub(super) fn body(&mut self) -> io::Result<()> {
        let mut depth = 1_u64;
        let mut quoted = false;
        let mut comment = false;
        let mut escape_digits = 0;

        while let Some(byte) = self.take()? {
            if comment {
                comment = byte != b'\n';
                continue;
            }

            if escape_digits != 0 {
                if escape_digits == 2 && byte == b'\\' {
                    escape_digits = 0;
                    continue;
                }

                if !byte.is_ascii_hexdigit() {
                    return Err(invalid(self.offset - 1, "Expected an LLVM hex escape"));
                }

                escape_digits -= 1;
                continue;
            }

            if quoted {
                match byte {
                    b'"' => quoted = false,
                    b'\\' => escape_digits = 2,
                    b'\n' | b'\r' => {
                        return Err(invalid(self.offset - 1, "Expected an escaped LLVM newline"));
                    }
                    _ => {}
                }

                continue;
            }

            match byte {
                b';' => comment = true,
                b'"' => quoted = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;

                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        Err(invalid(
            self.offset,
            "Expected a closing LLVM body brace, got EOF",
        ))
    }
}

impl Token {
    /// Decodes LLVM global names without demangling or lossy UTF-8 conversion.
    pub(super) fn symbol(mut self) -> io::Result<String> {
        // LLVM names use @[-a-zA-Z$._][-a-zA-Z$._0-9]*, numeric slots, or quoted bytes.
        // Quoted escapes encode one byte with two hex digits, including an escaped quote.
        // LLVM also prints a literal backslash as \\, as the definitions.ll round trip verifies.
        // See https://llvm.org/docs/LangRef.html#identifiers.
        if self.bytes.first() != Some(&b'@') || self.bytes.len() == 1 {
            return Err(invalid(self.offset, "Expected an LLVM global symbol"));
        }

        let quoted = self.bytes[1] == b'"';

        if !quoted
            && self.bytes[1].is_ascii_digit()
            && !self.bytes[1..].iter().all(u8::is_ascii_digit)
        {
            return Err(invalid(
                self.offset,
                "Expected a numeric LLVM slot or a named symbol",
            ));
        }

        let end = self.bytes.len() - usize::from(quoted);
        let mut read = if quoted { 2 } else { 1 };
        let mut write = 0;

        while read < end {
            let mut byte = self.bytes[read];
            read += 1;

            if quoted && byte == b'\\' {
                if self.bytes[read] == b'\\' {
                    read += 1;
                } else {
                    // The tokenizer validated both digits before constructing this token.
                    byte = (hex(self.bytes[read]) << 4) | hex(self.bytes[read + 1]);
                    read += 2;
                }
            }

            self.bytes[write] = byte;
            write += 1;
        }

        self.bytes.truncate(write);

        String::from_utf8(self.bytes)
            .map_err(|_| invalid(self.offset, "Expected a UTF-8 LLVM symbol"))
    }
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$' | b'-')
}

fn hex(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
    }
}

pub(super) fn matching_close(byte: u8) -> Option<u8> {
    match byte {
        b'(' => Some(b')'),
        b'[' => Some(b']'),
        b'{' => Some(b'}'),
        b'<' => Some(b'>'),
        _ => None,
    }
}

pub(super) fn invalid(offset: u64, message: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        format!("{message} at byte {offset}"),
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor, Read};

    use super::super::tests::ShortReader;
    use super::*;

    #[track_caller]
    fn decode(input: &[u8], max_read: usize) -> io::Result<String> {
        let mut stream = Stream::new(ShortReader::new(input, max_read));
        stream.begin_header(0);

        stream.required_token()?.symbol()
    }

    #[test]
    fn symbols_across_buffer_boundaries() {
        let cases: &[(&[u8], &str)] = &[
            (b"@_RNv.foo$-1", "_RNv.foo$-1"),               // Named symbol.
            (b"@123", "123"),                               // Numeric slot.
            (br#"@"a\22b\5cc\01\C3\A9""#, "a\"b\\c\u{1}é"), // Escaped bytes.
            (br#"@"@;{}() space""#, "@;{}() space"),        // Quoted punctuation.
            (br#"@"a\\b\22""#, "a\\b\""),                   // Disassembler backslash spelling.
        ];

        for &(input, expected) in cases {
            for max_read in [1, 2, 7, 8192] {
                assert_eq!(decode(input, max_read).unwrap(), expected);
            }
        }
    }

    #[test]
    fn malformed_symbols_have_offsets() {
        for input in [
            &b"@"[..],         // Missing name.
            b"@123abc",        // Invalid numeric slot.
            br#"@"bad\q0""#,   // Invalid hex escape.
            br#"@"bad\2""#,    // Short escape.
            br#"@"bad"#,       // Missing quote.
            br#"@"\FF""#,      // Invalid UTF-8.
            b"@\"bad\nname\"", // Unescaped newline.
        ] {
            let error = decode(input, 1).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData);
            assert!(error.to_string().contains("at byte"));
        }
    }

    #[test]
    fn header_budget_counts_whitespace_and_token_bytes() {
        for length in [HEADER_LIMIT - 1, HEADER_LIMIT, HEADER_LIMIT + 1] {
            for byte in *b" a" {
                let input = std::io::repeat(byte).take(length);
                let mut stream = Stream::new(BufReader::new(input));
                stream.begin_header(0);
                let result = stream.token(false);

                assert_eq!(result.is_ok(), length <= HEADER_LIMIT);
                assert!(stream.buffer_peaks().1 <= HEADER_LIMIT as usize);

                if byte == b'a' {
                    assert_eq!(stream.buffer_peaks().1, HEADER_LIMIT as usize);
                }
            }
        }
    }

    #[test]
    fn unrelated_line_and_body_use_fixed_storage() {
        let length = HEADER_LIMIT * 8;
        let line = std::io::repeat(b'x').take(length).chain(Cursor::new(b"\n"));
        let mut stream = Stream::new(BufReader::new(line));
        stream.skip_line().unwrap();
        assert_eq!(stream.offset(), length + 1);
        assert_eq!(stream.buffer_peaks(), (CHUNK_SIZE, 0, 0));

        let body = std::io::repeat(b' ').take(length).chain(Cursor::new(b"}"));
        let mut stream = Stream::new(BufReader::new(body));
        stream.body().unwrap();
        assert_eq!(stream.offset(), length + 1);
        assert_eq!(stream.buffer_peaks(), (CHUNK_SIZE, 0, 0));
    }

    #[test]
    fn body_ignores_quoted_and_commented_braces() {
        let input = b"\n; } define @fake {\n %x = insertvalue {i8, i8} poison, i8 0, 0\n call void asm \"{\\22}\", \"\"()\n ret void\n}tail";
        let mut stream = Stream::new(ShortReader::new(input, 1));
        stream.body().unwrap();

        assert_eq!(stream.offset(), (input.len() - 4) as u64);
        assert_eq!(stream.peek().unwrap(), Some(b't'));
    }

    #[test]
    fn truncated_bodies_fail() {
        for input in ["", "ret void", "{ }", "\"unterminated}", ";}"] {
            let mut stream = Stream::new(input.as_bytes());
            assert_eq!(stream.body().unwrap_err().kind(), ErrorKind::InvalidData);
        }
    }

    #[test]
    fn offsets_remain_u64() {
        let mut stream = Stream::new(&b"word"[..]);
        stream.offset = u64::from(u32::MAX) + 10;
        stream.begin_header(stream.offset());
        let token = stream.required_token().unwrap();

        assert_eq!(token.offset, u64::from(u32::MAX) + 10);
        assert_eq!(stream.offset(), u64::from(u32::MAX) + 14);

        let mut stream = Stream::new(&b"x"[..]);
        stream.offset = u64::MAX;
        assert_eq!(stream.take().unwrap_err().kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn group_nesting_is_bounded_and_checked() {
        for input in ["[<{i32}>])", "[])", ")"] {
            let mut stream = Stream::new(input.as_bytes());
            stream.begin_header(0);
            stream.group(b'(').unwrap();
        }

        for input in ["[)", "", "([)]"] {
            let mut stream = Stream::new(input.as_bytes());
            stream.begin_header(0);
            assert_eq!(
                stream.group(b'(').unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        let input = std::io::repeat(b'(').take(HEADER_LIMIT + 1);
        let mut stream = Stream::new(BufReader::new(input));
        stream.begin_header(0);
        stream.expect(b"(").unwrap();
        let error = stream.group(b'(').unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("1 MiB"));
        assert_eq!(stream.buffer_peaks().2, HEADER_LIMIT as usize);
    }
}
