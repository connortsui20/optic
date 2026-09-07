//! Reads LLVM header tokens and body boundaries with bounded storage.
//!
//! Header budgets count input bytes, including whitespace and comments. Body scans retain no
//! tokens. The tokenizer validates quoted bytes before symbol decoding relies on their layout.

use std::io::{self, BufRead, ErrorKind};

/// The header byte budget from the [show contract], independent of LLVM language limits.
///
/// [show contract]: https://github.com/connortsui20/optic/blob/planning/docs/design/show.md
pub(super) const HEADER_LIMIT: u64 = 1024 * 1024;
const CHUNK_SIZE: usize = 8192;

/// Fixed-buffer input that tracks consumed bytes independently of reader lookahead.
///
/// An active header budget bounds tokens and delimiter stacks. Body scans and unrelated lines use
/// only the fixed chunk. Reading ahead into that chunk does not consume the header budget.
pub(super) struct Stream<R> {
    reader: R,
    chunk: [u8; CHUNK_SIZE],
    /// The next unread chunk index, always at or before the filled length.
    position: usize,
    /// The filled prefix of the fixed chunk.
    length: usize,
    /// The number of bytes consumed from the complete input, not just the current chunk.
    offset: u64,
    /// A consumed header start whose distance from the offset is at most the header limit.
    header_start: Option<u64>,
    #[cfg(test)]
    peak_token_capacity: usize,
    #[cfg(test)]
    peak_group_capacity: usize,
}

/// A header token whose quotes and escapes were validated by [`Stream::read_token`].
///
/// The bytes **must** retain that validated form until [`Self::decode_symbol`] consumes them.
/// Decoding relies on closing quotes and complete escapes when it indexes these bytes.
pub(super) struct Token {
    /// The complete token, including its prefix and quotes.
    pub(super) bytes: Vec<u8>,
    /// The byte offset of the first token byte.
    pub(super) offset: u64,
}

impl<R: BufRead> Stream<R> {
    /// Starts an input stream at byte zero with no active header budget.
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

    /// Returns the offset of the next byte that will be consumed.
    pub(super) fn offset(&self) -> u64 {
        self.offset
    }

    /// Starts the header budget, including any prefix already consumed by the scanner.
    ///
    /// The start **must** be at or before [`Self::offset`]. The consumed prefix **must** fit within
    /// [`HEADER_LIMIT`] so subsequent reads can enforce the remaining budget.
    pub(super) fn begin_header(&mut self, start: u64) {
        self.header_start = Some(start);
    }

    /// Removes the header budget before streaming a body or unrelated line.
    pub(super) fn end_header(&mut self) {
        self.header_start = None;
    }

    /// Reads ahead without consuming a byte or changing the header budget.
    pub(super) fn peek(&mut self) -> io::Result<Option<u8>> {
        if self.position == self.length {
            self.refill()?;
        }

        Ok((self.position < self.length).then(|| self.chunk[self.position]))
    }

    /// Refills an exhausted chunk without changing the absolute offset or header budget.
    fn refill(&mut self) -> io::Result<()> {
        self.length = loop {
            match self.reader.read(&mut self.chunk) {
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                result => break result?,
            }
        };
        self.position = 0;

        Ok(())
    }

    /// Consumes one byte while enforcing the active header and absolute-offset bounds.
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

    /// Consumes through the next newline or EOF without retaining line content.
    pub(super) fn skip_line(&mut self) -> io::Result<()> {
        while let Some(byte) = self.take()? {
            if byte == b'\n' {
                break;
            }
        }

        Ok(())
    }

    /// Reads one token, optionally treating a newline as the end of a header.
    ///
    /// A header budget **must** be active. Comments and whitespace count against that budget even
    /// when they produce no token. Quoted tokens contain only complete, validated escapes.
    pub(super) fn read_token(&mut self, stop_at_newline: bool) -> io::Result<Option<Token>> {
        assert!(
            self.header_start.is_some(),
            "header tokens require an active byte budget"
        );

        while let Some(byte) = self.peek()? {
            let ended_line = match byte {
                b';' => {
                    self.skip_line()?;

                    true
                }
                byte if byte.is_ascii_whitespace() => {
                    self.take()?;

                    byte == b'\n'
                }
                _ => break,
            };

            if stop_at_newline && ended_line {
                return Ok(None);
            }
        }

        let offset = self.offset;
        let Some(first) = self.take()? else {
            return Ok(None);
        };

        let mut bytes = vec![first];

        // Retain the result until the test seam observes allocations from rejected tokens too.
        let result = self.complete_token(&mut bytes);

        #[cfg(test)]
        {
            self.peak_token_capacity = self.peak_token_capacity.max(bytes.capacity());
        }

        result?;

        Ok(Some(Token { bytes, offset }))
    }

    /// Reads across line boundaries and rejects EOF before the next token.
    pub(super) fn read_required_token(&mut self) -> io::Result<Token> {
        self.read_token(false)?
            .ok_or_else(|| invalid(self.offset, "Expected an LLVM header token, got EOF"))
    }

    /// Consumes one token and requires its exact encoded bytes.
    pub(super) fn expect(&mut self, expected: &[u8]) -> io::Result<()> {
        let token = self.read_required_token()?;

        if token.bytes != expected {
            return Err(invalid(
                token.offset,
                format!("Expected LLVM token {}", String::from_utf8_lossy(expected)),
            ));
        }

        Ok(())
    }

    /// Completes a token whose first byte is already present in the buffer.
    fn complete_token(&mut self, bytes: &mut Vec<u8>) -> io::Result<()> {
        let first = bytes[0];

        if first == b'"' {
            return self.complete_quoted_token(bytes);
        }

        if matches!(first, b'@' | b'%' | b'$') && self.peek()? == Some(b'"') {
            bytes.push(self.take()?.expect("peek established the opening quote"));

            return self.complete_quoted_token(bytes);
        }

        if !is_word(first) && !matches!(first, b'@' | b'%' | b'$' | b'!') {
            return Ok(());
        }

        while self.peek()?.is_some_and(is_word) {
            bytes.push(self.take()?.expect("peek established the next token byte"));
        }

        Ok(())
    }

    /// Appends through a closing quote after the caller has consumed the opening quote.
    fn complete_quoted_token(&mut self, bytes: &mut Vec<u8>) -> io::Result<()> {
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
                b'\\' => self.complete_quoted_escape(bytes)?,
                _ => {}
            }
        }
    }

    /// Appends the second backslash or two hex digits after a consumed escape marker.
    fn complete_quoted_escape(&mut self, bytes: &mut Vec<u8>) -> io::Result<()> {
        if self.peek()? == Some(b'\\') {
            bytes.push(
                self.take()?
                    .expect("peek established the escaped backslash"),
            );

            return Ok(());
        }

        for _ in 0..2 {
            let digit = self
                .take()?
                .ok_or_else(|| invalid(self.offset, "Expected two LLVM escape digits, got EOF"))?;

            if !digit.is_ascii_hexdigit() {
                return Err(invalid(self.offset - 1, "Expected an LLVM hex escape"));
            }

            bytes.push(digit);
        }

        Ok(())
    }

    /// Consumes a balanced header group without recursion.
    ///
    /// The caller **must** consume the opening delimiter first and keep the header budget active.
    pub(super) fn consume_group(&mut self, opening: u8) -> io::Result<()> {
        let mut closing =
            vec![matching_close(opening).expect("group callers supply an opening delimiter")];

        while let Some(&expected) = closing.last() {
            #[cfg(test)]
            {
                self.peak_group_capacity = self.peak_group_capacity.max(closing.capacity());
            }

            let token = self.read_required_token()?;

            if token.bytes.len() != 1 {
                continue;
            }

            let byte = token.bytes[0];

            if let Some(close) = matching_close(byte) {
                closing.push(close);

                continue;
            }

            if !matches!(byte, b')' | b']' | b'}' | b'>') {
                continue;
            }

            if byte != expected {
                return Err(invalid(token.offset, "Expected a matching LLVM delimiter"));
            }

            closing.pop();
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
    ///
    /// Quotes and comments hide braces from the nesting count. The header budget **must** be off
    /// because function bodies have no size limit. EOF before the closing brace returns an error.
    pub(super) fn skip_body(&mut self) -> io::Result<()> {
        let mut depth = 1_u64;
        let mut quoted = false;
        let mut comment = false;

        // A nonzero count occurs only inside a quote, after an escape's opening backslash.
        let mut escape_digits = 0;

        while let Some(byte) = self.take()? {
            if comment {
                comment = byte != b'\n';

                continue;
            }

            if escape_digits == 2 && byte == b'\\' {
                escape_digits = 0;

                continue;
            }

            if escape_digits != 0 && !byte.is_ascii_hexdigit() {
                return Err(invalid(self.offset - 1, "Expected an LLVM hex escape"));
            }

            if escape_digits != 0 {
                escape_digits -= 1;

                continue;
            }

            if quoted && matches!(byte, b'\n' | b'\r') {
                return Err(invalid(self.offset - 1, "Expected an escaped LLVM newline"));
            }

            if quoted && byte == b'"' {
                quoted = false;

                continue;
            }

            if quoted && byte == b'\\' {
                escape_digits = 2;

                continue;
            }

            if quoted {
                continue;
            }

            match byte {
                b';' => comment = true,
                b'"' => quoted = true,
                b'{' => depth += 1,
                b'}' if depth == 1 => return Ok(()),
                b'}' => depth -= 1,
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
    pub(super) fn decode_symbol(mut self) -> io::Result<String> {
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

        // Decoding never expands an escape, so writes stay behind unread bytes in the same buffer.
        while read < end {
            let mut byte = self.bytes[read];
            read += 1;

            if quoted && byte == b'\\' {
                byte = self.decode_escape(&mut read);
            }

            self.bytes[write] = byte;
            write += 1;
        }

        self.bytes.truncate(write);

        String::from_utf8(self.bytes)
            .map_err(|_| invalid(self.offset, "Expected a UTF-8 LLVM symbol"))
    }

    /// Decodes a validated escape after the cursor has consumed its opening backslash.
    fn decode_escape(&self, read: &mut usize) -> u8 {
        let first = self.bytes[*read];
        *read += 1;

        if first == b'\\' {
            return first;
        }

        // The tokenizer validated both digits before constructing this token.
        let byte = (hex(first) << 4) | hex(self.bytes[*read]);
        *read += 1;

        byte
    }
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$' | b'-')
}

/// Converts a digit already checked by the quoted-token reader.
fn hex(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => byte - b'A' + 10,
    }
}

/// Classifies an opening delimiter without treating another byte as malformed input.
pub(super) fn matching_close(byte: u8) -> Option<u8> {
    match byte {
        b'(' => Some(b')'),
        b'[' => Some(b']'),
        b'{' => Some(b'}'),
        b'<' => Some(b'>'),
        _ => None,
    }
}

/// Adds the byte offset to a malformed LLVM input error.
pub(super) fn invalid(offset: u64, message: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        format!("{message} at byte {offset}"),
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor, Read};

    use super::super::tests::READ_LIMITS;
    use super::super::tests::ShortReader;
    use super::*;

    #[track_caller]
    fn decode(input: &[u8], max_read: usize) -> io::Result<String> {
        let mut stream = Stream::new(ShortReader::new(input, max_read));
        stream.begin_header(0);

        stream.read_required_token()?.decode_symbol()
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
            for max_read in READ_LIMITS {
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
        for length in [
            HEADER_LIMIT - 1, // The length is below the limit.
            HEADER_LIMIT,     // The length equals the limit.
            HEADER_LIMIT + 1, // The length exceeds the limit.
        ] {
            for byte in *b" a" {
                let input = std::io::repeat(byte).take(length);
                let mut stream = Stream::new(BufReader::new(input));
                stream.begin_header(0);
                let result = stream.read_token(false);

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
        stream.skip_body().unwrap();

        assert_eq!(stream.offset(), length + 1);
        assert_eq!(stream.buffer_peaks(), (CHUNK_SIZE, 0, 0));
    }

    #[test]
    fn body_ignores_quoted_and_commented_braces() {
        let input = b"\n; } define @fake {\n \
                      %x = insertvalue {i8, i8} poison, i8 0, 0\n \
                      call void asm \"{\\22}\", \"\"()\n ret void\n}tail";
        let mut stream = Stream::new(ShortReader::new(input, 1));
        stream.skip_body().unwrap();

        assert_eq!(stream.offset(), (input.len() - 4) as u64);
        assert_eq!(stream.peek().unwrap(), Some(b't'));
    }

    #[test]
    fn truncated_bodies_fail() {
        for input in [
            "",                // The body is empty.
            "ret void",        // The closing brace is missing.
            "{ }",             // Only the nested brace closes.
            "\"unterminated}", // An unclosed quote hides the brace.
            ";}",              // A comment hides the brace.
        ] {
            let mut stream = Stream::new(input.as_bytes());

            assert_eq!(
                stream.skip_body().unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn offsets_remain_u64() {
        let mut stream = Stream::new(&b"word"[..]);
        stream.offset = u64::from(u32::MAX) + 10;
        stream.begin_header(stream.offset());
        let token = stream.read_required_token().unwrap();

        assert_eq!(token.offset, u64::from(u32::MAX) + 10);
        assert_eq!(stream.offset(), u64::from(u32::MAX) + 14);

        let mut stream = Stream::new(&b"x"[..]);
        stream.offset = u64::MAX;

        assert_eq!(stream.take().unwrap_err().kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn group_nesting_is_bounded_and_checked() {
        for input in [
            "[<{i32}>])", // The group nests delimiter kinds.
            "[])",        // The nested group is empty.
            ")",          // The outer group is empty.
        ] {
            let mut stream = Stream::new(input.as_bytes());
            stream.begin_header(0);
            stream.consume_group(b'(').unwrap();
        }

        for input in [
            "[)",   // The closing delimiter does not match.
            "",     // The outer closing delimiter is missing.
            "([)]", // The nesting order is invalid.
        ] {
            let mut stream = Stream::new(input.as_bytes());
            stream.begin_header(0);

            assert_eq!(
                stream.consume_group(b'(').unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        let input = std::io::repeat(b'(').take(HEADER_LIMIT + 1);
        let mut stream = Stream::new(BufReader::new(input));
        stream.begin_header(0);
        stream.expect(b"(").unwrap();
        let error = stream.consume_group(b'(').unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("1 MiB"));
        assert_eq!(stream.buffer_peaks().2, HEADER_LIMIT as usize);
    }
}
