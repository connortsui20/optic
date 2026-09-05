//! Supplies short reads for the parser's buffer-boundary tests.
//!
//! A large read can bypass a [`std::io::BufReader`] buffer, so its capacity does not establish a read
//! limit. [`ShortReader`] limits the bytes that each read returns, regardless of the requested size.

use std::io::{self, BufRead, Read};

/// Restricts reads and buffered slices to a positive byte limit.
pub(super) struct ShortReader<'a> {
    input: &'a [u8],
    max_read: usize,
}

impl<'a> ShortReader<'a> {
    /// Requires a positive limit so remaining input cannot look like EOF.
    #[track_caller]
    pub(super) fn new(input: &'a [u8], max_read: usize) -> Self {
        assert!(max_read > 0, "short reads require a positive byte limit");

        Self { input, max_read }
    }
}

impl Read for ShortReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(self.max_read);

        self.input.read(&mut buffer[..length])
    }
}

impl BufRead for ShortReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Ok(&self.input[..self.input.len().min(self.max_read)])
    }

    fn consume(&mut self, amount: usize) {
        self.input.consume(amount);
    }
}

#[test]
fn read_limits_apply_to_large_requests() {
    let input = b"abcdefghijklmnop";

    for max_read in [1, 2, 7, 8192] {
        let mut reader = ShortReader::new(input, max_read);
        let mut buffer = [0; 8192];
        assert_eq!(reader.read(&mut []).unwrap(), 0);

        for expected in input.chunks(max_read) {
            let length = reader.read(&mut buffer).unwrap();
            assert_eq!(length, expected.len());
            assert_eq!(&buffer[..length], expected);
        }

        assert_eq!(reader.read(&mut buffer).unwrap(), 0);
    }
}
