//! Keeps [`CaptureId`](crate::CaptureId) generation and parsing on one reverse-hexadecimal grammar.
//!
//! The distinct alphabet separates capture IDs from future instance IDs when a boundary cannot
//! preserve their Rust types.

const DIGITS: &[u8; 16] = b"zyxwvutsrqponmlk";

/// Encodes each byte as two reverse-hexadecimal digits, with the high nibble first.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }

    encoded
}

/// Accepts complete reverse-hexadecimal byte pairs, including an empty sequence.
pub(crate) fn is_canonical(value: &str) -> bool {
    value.len().is_multiple_of(2) && value.bytes().all(|digit| (b'k'..=b'z').contains(&digit))
}

#[cfg(test)]
mod tests {
    use super::encode;
    use super::is_canonical;

    #[test]
    fn maps_nibbles_to_reverse_hexadecimal_digits() {
        let bytes = [
            0x01, // Map to `zy`.
            0x23, // Map to `xw`.
            0x45, // Map to `vu`.
            0x67, // Map to `ts`.
            0x89, // Map to `rq`.
            0xab, // Map to `po`.
            0xcd, // Map to `nm`.
            0xef, // Map to `lk`.
        ];

        assert_eq!(encode(&bytes), "zyxwvutsrqponmlk");
        assert!(is_canonical("zyxwvutsrqponmlk"));
    }

    #[test]
    fn rejects_each_malformed_text_shape() {
        for value in [
            "z",  // Reject an incomplete byte pair.
            "zj", // Reject ASCII immediately below the digit range.
            "z0", // Reject an unrelated ASCII digit.
        ] {
            assert!(!is_canonical(value), "accepted {value}");
        }
    }
}
