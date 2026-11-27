//! HUFF/CDIC compression: Huffman codes for phrases from dictionaries,
//! which may themselves be compressed (older Kindle books from kindlegen).

use akkhor_core::{Error, Result};

use crate::palmdb::be32;

/// The deepest a phrase may refer to other compressed phrases.
const MAX_DEPTH: usize = 32;

pub(crate) struct Huff {
    /// Per first byte of a code: length, whether it ends there, max code.
    dict1: Vec<(u32, bool, u32)>,
    mincode: [u32; 33],
    maxcode: [u32; 33],
    /// Phrases: bytes and whether they are ready (not themselves coded).
    phrases: Vec<(Vec<u8>, bool)>,
}

impl Huff {
    pub fn new(huff: &[u8], cdics: &[&[u8]]) -> Result<Self> {
        if !huff.starts_with(b"HUFF") || huff.len() < 24 {
            return Err(Error::malformed("bad HUFF record"));
        }
        let off1 = be32(huff, 8) as usize;
        let off2 = be32(huff, 12) as usize;
        if huff.len() < off1 + 1024 || huff.len() < off2 + 256 {
            return Err(Error::malformed("HUFF tables run past the record"));
        }
        let mut dict1 = Vec::with_capacity(256);
        for i in 0..256 {
            let v = be32(huff, off1 + i * 4);
            let len = v & 0x1f;
            let term = v & 0x80 != 0;
            let mut max = v >> 8;
            if len == 0 || (len <= 8 && !term) {
                return Err(Error::malformed("bad HUFF code table"));
            }
            max = (((u64::from(max) + 1) << (32 - len)) - 1) as u32;
            dict1.push((len, term, max));
        }
        let mut mincode = [0u32; 33];
        let mut maxcode = [0u32; 33];
        for len in 1..=32usize {
            let lo = u64::from(be32(huff, off2 + (len - 1) * 8));
            let hi = u64::from(be32(huff, off2 + (len - 1) * 8 + 4));
            mincode[len] = (lo << (32 - len)) as u32;
            maxcode[len] = (((hi + 1) << (32 - len)) - 1) as u32;
        }
        let mut phrases = Vec::new();
        for c in cdics {
            if !c.starts_with(b"CDIC") || c.len() < 16 {
                return Err(Error::malformed("bad CDIC record"));
            }
            let total = be32(c, 8) as usize;
            let bits = be32(c, 12).min(31);
            let n = (1usize << bits).min(total.saturating_sub(phrases.len()));
            for k in 0..n {
                let off = c
                    .get(16 + k * 2..18 + k * 2)
                    .map_or(0, |s| u16::from_be_bytes([s[0], s[1]]) as usize);
                let at = 16 + off;
                let blen = c
                    .get(at..at + 2)
                    .map_or(0, |s| u16::from_be_bytes([s[0], s[1]]));
                let len = (blen & 0x7fff) as usize;
                let bytes = c.get(at + 2..at + 2 + len).unwrap_or(&[]).to_vec();
                phrases.push((bytes, blen & 0x8000 != 0));
            }
        }
        Ok(Self {
            dict1,
            mincode,
            maxcode,
            phrases,
        })
    }

    pub fn decompress(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<()> {
        self.unpack(data, out, 0)
    }

    fn unpack(&mut self, data: &[u8], out: &mut Vec<u8>, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Error::malformed("HUFF phrases nest too deep"));
        }
        let mut bits_left = data.len() as i64 * 8;
        let mut padded = data.to_vec();
        padded.extend_from_slice(&[0; 8]);
        let word = |pos: usize| -> u64 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&padded[pos..pos + 8]);
            u64::from_be_bytes(b)
        };
        let mut pos = 0;
        let mut x = word(0);
        let mut n: i32 = 32;
        loop {
            if n <= 0 {
                pos += 4;
                if pos + 8 > padded.len() {
                    break;
                }
                x = word(pos);
                n += 32;
            }
            let code = ((x >> n) & 0xffff_ffff) as u32;
            let (mut len, term, mut max) = self.dict1[(code >> 24) as usize];
            if !term {
                while len < 32 && code < self.mincode[len as usize] {
                    len += 1;
                }
                max = self.maxcode[len as usize];
            }
            n -= len as i32;
            bits_left -= i64::from(len);
            if bits_left < 0 {
                break;
            }
            let r = ((u64::from(max).wrapping_sub(u64::from(code))) >> (32 - len)) as usize;
            let Some((phrase, ready)) = self.phrases.get(r).cloned() else {
                return Err(Error::malformed("HUFF code outside the dictionary"));
            };
            if ready {
                out.extend_from_slice(&phrase);
            } else {
                // Mark it while expanding, so a phrase that refers to itself
                // fails instead of looping.
                self.phrases[r] = (Vec::new(), true);
                let mut expanded = Vec::new();
                let done = self.unpack(&phrase, &mut expanded, depth + 1);
                self.phrases[r] = (expanded.clone(), true);
                done?;
                out.extend_from_slice(&expanded);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A HUFF record where every code is one byte: byte `b` is phrase
    /// `255 - b`.
    fn huff_record() -> Vec<u8> {
        let mut h = b"HUFF".to_vec();
        h.extend_from_slice(&24u32.to_be_bytes());
        h.extend_from_slice(&24u32.to_be_bytes()); // table 1
        h.extend_from_slice(&(24 + 1024u32).to_be_bytes()); // table 2
        h.extend_from_slice(&[0; 8]);
        for _ in 0..256 {
            // length 8, terminal, max code 255
            h.extend_from_slice(&((255u32 << 8) | 0x80 | 8).to_be_bytes());
        }
        h.extend_from_slice(&[0; 256]);
        h
    }

    fn cdic(phrases: &[(&[u8], bool)]) -> Vec<u8> {
        let mut c = b"CDIC".to_vec();
        c.extend_from_slice(&16u32.to_be_bytes());
        c.extend_from_slice(&(phrases.len() as u32).to_be_bytes());
        c.extend_from_slice(&8u32.to_be_bytes());
        let mut offsets = Vec::new();
        let mut body = Vec::new();
        let table = phrases.len() * 2;
        for (p, ready) in phrases {
            offsets.push((table + body.len()) as u16);
            let flag = if *ready { 0x8000 } else { 0 };
            body.extend_from_slice(&((p.len() as u16) | flag).to_be_bytes());
            body.extend_from_slice(p);
        }
        for o in offsets {
            c.extend_from_slice(&o.to_be_bytes());
        }
        c.extend_from_slice(&body);
        c
    }

    #[test]
    fn phrases_and_nested_phrases() {
        // Phrase 0 "Akkhor ", phrase 1 "বই", phrase 2 is itself coded: the
        // codes for phrases 1 then 0 (bytes 254, 255).
        let mut phrases: Vec<(&[u8], bool)> = vec![
            (b"Akkhor ", true),
            ("বই".as_bytes(), true),
            (&[254, 255], false),
        ];
        phrases.extend(std::iter::repeat_n((&b"?"[..], true), 253));
        let mut h = Huff::new(&huff_record(), &[&cdic(&phrases)]).unwrap();
        let mut out = Vec::new();
        h.decompress(&[255, 254, 253, 255], &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "Akkhor বইবইAkkhor Akkhor ");
    }

    #[test]
    fn a_phrase_that_refers_to_itself_ends() {
        let mut phrases: Vec<(&[u8], bool)> = vec![(&[255], false)];
        phrases.extend(std::iter::repeat_n((&b"?"[..], true), 255));
        let mut h = Huff::new(&huff_record(), &[&cdic(&phrases)]).unwrap();
        let mut out = Vec::new();
        let _ = h.decompress(&[255, 255], &mut out);
        assert!(out.len() < 16);
    }
}
