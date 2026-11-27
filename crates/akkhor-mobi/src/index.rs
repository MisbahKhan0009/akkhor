//! INDX records: the indexes of a MOBI file (table of contents, and in KF8
//! the skeleton and fragment tables). Each entry is a label and a map of
//! tags to numbers, described by a TAGX table; strings live in CNCX
//! records after the index.

use std::collections::BTreeMap;

use crate::palmdb::{PalmDb, be16, be32};

/// The most entries one index may produce (a novel's has hundreds).
const MAX_ENTRIES: usize = 200_000;

pub(crate) struct Entry {
    pub label: Vec<u8>,
    pub tags: BTreeMap<u8, Vec<u32>>,
}

impl Entry {
    pub fn tag(&self, t: u8, i: usize) -> Option<u32> {
        self.tags.get(&t).and_then(|v| v.get(i)).copied()
    }
}

pub(crate) struct Index {
    pub entries: Vec<Entry>,
    /// CNCX records, concatenated (offsets index into them by 0x10000 per
    /// record).
    cncx: Vec<Vec<u8>>,
}

impl Index {
    /// Reads the index whose header is record `first` (relative to `base`).
    pub fn read(db: &PalmDb, base: usize, first: u32) -> Option<Index> {
        let at = base + first as usize;
        let head = db.record(at);
        if !head.starts_with(b"INDX") {
            return None;
        }
        let header_len = be32(head, 4) as usize;
        let records = be32(head, 24) as usize;
        let cncx_count = be32(head, 52) as usize;
        let tagx = head.get(header_len..)?;
        let (control_bytes, table) = tag_table(tagx)?;
        let mut cncx = Vec::new();
        for j in 0..cncx_count.min(64) {
            cncx.push(
                db.record(at.saturating_add(records).saturating_add(1 + j))
                    .to_vec(),
            );
        }
        let mut entries = Vec::new();
        for i in 0..records.min(1024) {
            let rec = db.record(at + 1 + i);
            if !rec.starts_with(b"INDX") {
                continue;
            }
            let idxt = be32(rec, 20) as usize;
            // The count is the file's word: no more than the offset table
            // after IDXT can hold.
            let room = rec.len().saturating_sub(idxt.saturating_add(4)) / 2;
            let count = (be32(rec, 24) as usize).min(room).min(MAX_ENTRIES);
            let ordt = ordt_map(rec);
            let mut starts: Vec<usize> = (0..count)
                .map(|j| be16(rec, idxt + 4 + 2 * j) as usize)
                .collect();
            starts.push(idxt);
            for j in 0..count {
                if entries.len() >= MAX_ENTRIES {
                    break;
                }
                let (s, e) = (starts[j], starts[j + 1].min(rec.len()));
                if s >= e {
                    continue;
                }
                let n = rec[s] as usize;
                let Some(raw) = rec.get(s + 1..s + 1 + n) else {
                    continue;
                };
                let label = match &ordt {
                    Some(map) => raw
                        .iter()
                        .map(|&b| map.get(b as usize).copied().unwrap_or(u16::from(b)) as u8)
                        .collect(),
                    None => raw.to_vec(),
                };
                let tags = tag_map(control_bytes, &table, &rec[..e], s + 1 + n);
                entries.push(Entry { label, tags });
            }
        }
        Some(Index { entries, cncx })
    }

    /// The string at `offset` in the CNCX records.
    pub fn string(&self, offset: u32) -> Option<&[u8]> {
        let rec = self.cncx.get((offset >> 16) as usize)?;
        let mut at = (offset & 0xffff) as usize;
        let (len, used) = forward_varint(rec.get(at..)?)?;
        at += used;
        rec.get(at..at + len as usize)
    }
}

type Tag = (u8, u8, u8, u8);

fn tag_table(b: &[u8]) -> Option<(usize, Vec<Tag>)> {
    if !b.starts_with(b"TAGX") {
        return None;
    }
    let first_entry = be32(b, 4) as usize;
    let control_bytes = be32(b, 8) as usize;
    let mut table = Vec::new();
    let mut i = 12;
    while i + 4 <= first_entry.min(b.len()) {
        table.push((b[i], b[i + 1], b[i + 2], b[i + 3]));
        i += 4;
    }
    Some((control_bytes, table))
}

/// Labels may be stored as indexes into a character table (ORDT).
fn ordt_map(rec: &[u8]) -> Option<Vec<u16>> {
    let entries = be32(rec, 0xa8) as usize;
    let op2 = be32(rec, 0xb0) as usize;
    if entries == 0 || entries > 65536 || rec.get(op2..op2 + 4) != Some(b"ORDT") {
        return None;
    }
    Some((0..entries).map(|k| be16(rec, op2 + 4 + 2 * k)).collect())
}

fn tag_map(control_bytes: usize, table: &[Tag], b: &[u8], start: usize) -> BTreeMap<u8, Vec<u32>> {
    let mut found: Vec<(u8, Option<u32>, Option<u32>, u8)> = Vec::new();
    let mut control = 0;
    let mut data = start + control_bytes;
    for &(tag, per_entry, mask, end) in table {
        if end == 1 {
            control += 1;
            continue;
        }
        let Some(&cbyte) = b.get(start + control) else {
            break;
        };
        let value = cbyte & mask;
        if value == 0 || mask == 0 {
            continue;
        }
        if value == mask {
            if mask.count_ones() > 1 {
                let Some((bytes, used)) = forward_varint(b.get(data..).unwrap_or(&[])) else {
                    break;
                };
                data += used;
                found.push((tag, None, Some(bytes), per_entry));
            } else {
                found.push((tag, Some(1), None, per_entry));
            }
        } else {
            let shift = mask.trailing_zeros();
            found.push((tag, Some(u32::from(value >> shift)), None, per_entry));
        }
    }
    let mut out = BTreeMap::new();
    for (tag, count, bytes, per_entry) in found {
        let mut values = Vec::new();
        if let Some(count) = count {
            for _ in 0..(count as usize * per_entry.max(1) as usize).min(4096) {
                let Some((v, used)) = forward_varint(b.get(data..).unwrap_or(&[])) else {
                    break;
                };
                data += used;
                values.push(v);
            }
        } else if let Some(bytes) = bytes {
            let mut consumed = 0;
            while consumed < bytes as usize {
                let Some((v, used)) = forward_varint(b.get(data..).unwrap_or(&[])) else {
                    break;
                };
                data += used;
                consumed += used;
                values.push(v);
            }
        }
        out.insert(tag, values);
    }
    out
}

/// A forward variable-width number: 7 bits a byte, the last byte has the
/// high bit set. Returns the value and the bytes used.
pub(crate) fn forward_varint(b: &[u8]) -> Option<(u32, usize)> {
    let mut v: u32 = 0;
    for (i, &c) in b.iter().enumerate().take(5) {
        v = (v << 7) | u32::from(c & 0x7f);
        if c & 0x80 != 0 {
            return Some((v, i + 1));
        }
    }
    None
}
