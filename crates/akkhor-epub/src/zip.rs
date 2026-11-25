//! A minimal, read-only zip reader over a memory-mapped file.
//!
//! Opening reads only the central directory at the end of the archive (a few
//! KB even for a 500 MB comic or EPUB). An entry is located and inflated only
//! when asked for, so memory holds just the chapter or image being shown.

use std::collections::HashMap;

use akkhor_core::{Error, Result, Source};
use miniz_oxide::inflate::decompress_to_vec_with_limit;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    method: u16,
    compressed_size: u64,
    pub size: u64,
    local_header: u64,
}

pub struct ZipArchive {
    source: Source,
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

/// Refuse to inflate a single entry larger than this; protects against zip
/// bombs. Chapters and images are far smaller.
const MAX_ENTRY: usize = 256 << 20;

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64le(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn bad() -> Error {
    Error::malformed("damaged zip archive")
}

/// `data[at..at + len]`, with every addition checked: sizes and offsets come
/// from the file and may be hostile.
fn slice(data: &[u8], at: u64, len: u64) -> Option<&[u8]> {
    let start = usize::try_from(at).ok()?;
    let end = start.checked_add(usize::try_from(len).ok()?)?;
    data.get(start..end)
}

impl ZipArchive {
    pub fn open(source: Source) -> Result<Self> {
        let data = source.bytes();
        // End of central directory: in the last 64 KB + 22 bytes.
        let search_from = data.len().saturating_sub(65_557);
        let eocd = (search_from..data.len().saturating_sub(21))
            .rev()
            .find(|&i| data[i..i + 4] == *b"PK\x05\x06")
            .ok_or_else(bad)?;
        let mut count = u16le(data, eocd + 10).ok_or_else(bad)? as u64;
        let mut cd_offset = u32le(data, eocd + 16).ok_or_else(bad)? as u64;

        // Zip64: a locator sits right before the EOCD record.
        if (count == 0xFFFF || cd_offset == 0xFFFF_FFFF) && eocd >= 20 {
            let loc = eocd - 20;
            if data[loc..loc + 4] == *b"PK\x06\x07" {
                let z64 = u64le(data, loc + 8).ok_or_else(bad)? as usize;
                if data.get(z64..z64 + 4) == Some(b"PK\x06\x06") {
                    count = u64le(data, z64 + 32).ok_or_else(bad)?;
                    cd_offset = u64le(data, z64 + 48).ok_or_else(bad)?;
                }
            }
        }

        let mut entries = Vec::with_capacity(count.min(100_000) as usize);
        let mut pos = usize::try_from(cd_offset).map_err(|_| bad())?;
        for _ in 0..count {
            if slice(data, pos as u64, 46).is_none_or(|h| !h.starts_with(b"PK\x01\x02")) {
                return Err(bad());
            }
            let method = u16le(data, pos + 10).ok_or_else(bad)?;
            let mut compressed_size = u32le(data, pos + 20).ok_or_else(bad)? as u64;
            let mut size = u32le(data, pos + 24).ok_or_else(bad)? as u64;
            let name_len = u16le(data, pos + 28).ok_or_else(bad)? as usize;
            let extra_len = u16le(data, pos + 30).ok_or_else(bad)? as usize;
            let comment_len = u16le(data, pos + 32).ok_or_else(bad)? as usize;
            let mut local_header = u32le(data, pos + 42).ok_or_else(bad)? as u64;
            let name_bytes = slice(data, pos as u64 + 46, name_len as u64).ok_or_else(bad)?;
            let name = String::from_utf8_lossy(name_bytes).into_owned();

            // Zip64 extra field carries the real sizes and offset.
            let extra = slice(data, (pos + 46 + name_len) as u64, extra_len as u64).unwrap_or(&[]);
            let mut e = 0;
            while e + 4 <= extra.len() {
                let id = u16le(extra, e).unwrap_or(0);
                let len = u16le(extra, e + 2).unwrap_or(0) as usize;
                if id == 0x0001 {
                    let mut f = e + 4;
                    if size == 0xFFFF_FFFF {
                        size = u64le(extra, f).unwrap_or(size);
                        f += 8;
                    }
                    if compressed_size == 0xFFFF_FFFF {
                        compressed_size = u64le(extra, f).unwrap_or(compressed_size);
                        f += 8;
                    }
                    if local_header == 0xFFFF_FFFF {
                        local_header = u64le(extra, f).unwrap_or(local_header);
                    }
                }
                e += 4 + len;
            }

            entries.push(Entry {
                name,
                method,
                compressed_size,
                size,
                local_header,
            });
            pos += 46 + name_len + extra_len + comment_len;
        }

        let by_name = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), i))
            .collect();
        Ok(Self {
            source,
            entries,
            by_name,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Reads and inflates one entry.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let idx = *self.by_name.get(name).ok_or(Error::OutOfRange)?;
        let e = &self.entries[idx];
        let data = self.source.bytes();
        let header = slice(data, e.local_header, 30).ok_or_else(bad)?;
        if !header.starts_with(b"PK\x03\x04") {
            return Err(bad());
        }
        let lh = e.local_header as usize;
        let flags = u16le(data, lh + 6).ok_or_else(bad)?;
        if flags & 1 != 0 {
            return Err(Error::Protected);
        }
        let name_len = u16le(data, lh + 26).ok_or_else(bad)? as usize;
        let extra_len = u16le(data, lh + 28).ok_or_else(bad)? as usize;
        let start = (lh + 30 + name_len + extra_len) as u64;
        let raw = slice(data, start, e.compressed_size).ok_or_else(bad)?;
        match e.method {
            0 if raw.len() > MAX_ENTRY => Err(Error::malformed("zip entry too large")),
            0 => Ok(raw.to_vec()),
            8 => decompress_to_vec_with_limit(raw, (e.size as usize).min(MAX_ENTRY))
                .map_err(|_| bad()),
            m => Err(Error::malformed(format!("unsupported zip compression {m}"))),
        }
    }

    pub fn read_string(&self, name: &str) -> Result<String> {
        let bytes = self.read(name)?;
        Ok(match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        })
    }
}
