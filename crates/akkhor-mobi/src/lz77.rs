//! PalmDOC compression (a byte-pair LZ77 variant).

/// Decompresses one PalmDOC record, appending to `out`. Never reads or
/// copies outside what is there: a bad back-reference copies nothing.
pub(crate) fn decompress(data: &[u8], out: &mut Vec<u8>) {
    let start = out.len();
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        i += 1;
        match c {
            0x00 | 0x09..=0x7f => out.push(c),
            0x01..=0x08 => {
                let n = (c as usize).min(data.len() - i);
                out.extend_from_slice(&data[i..i + n]);
                i += n;
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
            0x80..=0xbf => {
                let Some(&c2) = data.get(i) else { break };
                i += 1;
                let pair = (u16::from(c) << 8) | u16::from(c2);
                let dist = ((pair >> 3) & 0x7ff) as usize;
                let len = (pair & 7) as usize + 3;
                let have = out.len() - start;
                if dist == 0 || dist > have {
                    continue;
                }
                for _ in 0..len {
                    let b = out[out.len() - dist];
                    out.push(b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_runs_spaces_and_copies() {
        let mut out = Vec::new();
        // "ab", 2 raw bytes, " c" packed, then copy distance 4, length 3.
        let dist_len: u16 = 0x8000 | (4 << 3);
        let mut data = vec![b'a', b'b', 0x02, 0xff, 0x01, 0xe3];
        data.extend_from_slice(&dist_len.to_be_bytes());
        decompress(&data, &mut out);
        assert_eq!(out, b"ab\xff\x01 c\xff\x01 ");
    }

    #[test]
    fn bad_references_copy_nothing() {
        let mut out = Vec::new();
        decompress(&[b'x', 0x80, 0x00, 0xbf, 0xff, 0x05], &mut out);
        assert_eq!(out, b"x");
    }
}
