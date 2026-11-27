//! Base64 as FB2 stores pictures: standard alphabet, whitespace anywhere,
//! padding optional.

pub(crate) fn decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_with_whitespace_and_padding() {
        assert_eq!(super::decode(b"QWtr\naG9y\r\n").unwrap(), b"Akkhor");
        assert_eq!(
            super::decode(b"4KaF4KaV4KeN4Ka3").unwrap(),
            "অক্ষ".as_bytes()
        );
        assert_eq!(super::decode(b"YQ==").unwrap(), b"a");
        assert!(super::decode(b"*!").is_none());
    }
}
