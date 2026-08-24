const DIGITS: &[u8; 16] = b"zyxwvutsrqponmlk";

pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }

    encoded
}

pub(crate) fn decode(value: &str) -> Option<Vec<u8>> {
    let mut digits = value.bytes();
    let mut decoded = Vec::with_capacity(value.len() / 2);

    while let Some(high) = digits.next() {
        let low = digits.next()?;
        let high = decode_digit(high)?;
        let low = decode_digit(low)?;

        decoded.push((high << 4) | low);
    }

    Some(decoded)
}

fn decode_digit(digit: u8) -> Option<u8> {
    (b'k'..=b'z').contains(&digit).then(|| b'z' - digit)
}

#[cfg(test)]
mod tests {
    use super::decode;
    use super::encode;

    #[test]
    fn maps_nibbles_to_reverse_hexadecimal_digits() {
        let bytes = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];

        assert_eq!(encode(&bytes), "zyxwvutsrqponmlk");
        assert_eq!(decode("zyxwvutsrqponmlk"), Some(bytes.to_vec()));
    }

    #[test]
    fn rejects_odd_length_and_non_reverse_hexadecimal_text() {
        assert_eq!(decode("z"), None);
        assert_eq!(decode("zj"), None);
        assert_eq!(decode("z0"), None);
    }
}
