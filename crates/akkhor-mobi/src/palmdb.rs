//! The Palm database container: a 78-byte header, then a list of record
//! offsets. Records are slices of the memory-mapped file.

use akkhor_core::{Error, Result, Source};

pub(crate) struct PalmDb {
    source: Source,
    /// Start of each record, then the file's end.
    bounds: Vec<usize>,
    /// Database type and creator, e.g. `BOOKMOBI` or `TEXtREAd`.
    pub kind: [u8; 8],
}

impl PalmDb {
    pub fn open(source: Source) -> Result<Self> {
        let b = source.bytes();
        if b.len() < 78 {
            return Err(Error::malformed("too short for a Palm database"));
        }
        let mut kind = [0u8; 8];
        kind.copy_from_slice(&b[60..68]);
        let n = u16::from_be_bytes([b[76], b[77]]) as usize;
        if b.len() < 78 + n * 8 {
            return Err(Error::malformed("record list runs past the file"));
        }
        let mut bounds = Vec::with_capacity(n + 1);
        let mut last = 78 + n * 8;
        for i in 0..n {
            let at = 78 + i * 8;
            let off = u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize;
            // Offsets must not go backwards or past the end; a broken entry
            // becomes an empty record rather than a wild slice.
            let off = off.clamp(last, b.len());
            bounds.push(off);
            last = off;
        }
        bounds.push(b.len());
        Ok(Self {
            source,
            bounds,
            kind,
        })
    }

    pub fn len(&self) -> usize {
        self.bounds.len() - 1
    }

    /// Record `i`, empty if there is no such record.
    pub fn record(&self, i: usize) -> &[u8] {
        if i + 1 >= self.bounds.len() {
            return &[];
        }
        &self.source.bytes()[self.bounds[i]..self.bounds[i + 1]]
    }
}

pub(crate) fn be16(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2)
        .map_or(0, |s| u16::from_be_bytes([s[0], s[1]]))
}

pub(crate) fn be32(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .map_or(0, |s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
