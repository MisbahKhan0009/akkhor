//! Record 0 of a MOBI section: the PalmDOC header, the MOBI header and
//! the EXTH metadata records. Offsets are from the start of the record.

use crate::palmdb::{be16, be32};

pub(crate) const NONE: u32 = 0xffff_ffff;

#[derive(Debug, Clone, Default)]
pub(crate) struct Header {
    /// 1: none, 2: PalmDOC, 17480: HUFF/CDIC.
    pub compression: u16,
    pub text_length: u32,
    pub text_records: u16,
    /// Non-zero: Kindle DRM.
    pub encryption: u16,
    /// 65001 (UTF-8) or 1252.
    pub encoding: u32,
    /// 6 or lower: MOBI 7; 8: KF8.
    pub version: u32,
    pub full_name: Option<String>,
    pub locale: u32,
    pub first_image: u32,
    pub huff_record: u32,
    pub huff_count: u32,
    pub extra_flags: u16,
    pub ncx_index: u32,
    /// KF8 only.
    pub fdst: u32,
    pub fragment_index: u32,
    pub skeleton_index: u32,
    pub exth: Exth,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Exth {
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub cover_offset: Option<u32>,
    pub thumb_offset: Option<u32>,
    /// Record number of the KF8 section's record 0 (combined files).
    pub kf8_boundary: Option<u32>,
    /// KF8: `kindle:embed:` URI of the cover.
    pub cover_uri: Option<String>,
}

impl Header {
    pub fn parse(r0: &[u8]) -> Option<Self> {
        if r0.len() < 16 {
            return None;
        }
        let mut h = Header {
            compression: be16(r0, 0),
            text_length: be32(r0, 4),
            text_records: be16(r0, 8),
            encryption: be16(r0, 12),
            encoding: 1252,
            first_image: NONE,
            huff_record: NONE,
            ncx_index: NONE,
            fdst: NONE,
            fragment_index: NONE,
            skeleton_index: NONE,
            ..Default::default()
        };
        if r0.get(16..20) != Some(b"MOBI") {
            // A plain PalmDOC book: no MOBI header.
            return Some(h);
        }
        let len = be32(r0, 20) as usize;
        let field = |at: usize| {
            if at + 4 <= 16 + len {
                be32(r0, at)
            } else {
                NONE
            }
        };
        h.encoding = be32(r0, 28);
        h.version = be32(r0, 36);
        let name_off = be32(r0, 84) as usize;
        let name_len = be32(r0, 88) as usize;
        h.full_name = r0
            .get(name_off..name_off.saturating_add(name_len))
            .map(|b| decode(b, h.encoding))
            .filter(|s| !s.trim().is_empty());
        h.locale = be32(r0, 92);
        h.first_image = field(108);
        h.huff_record = field(112);
        h.huff_count = field(116);
        if 16 + len >= 0xe4 {
            h.extra_flags = be16(r0, 0xf2);
        }
        if 16 + len >= 0xf8 {
            h.ncx_index = be32(r0, 0xf4);
        }
        if h.version >= 8 {
            h.fdst = field(0xc0);
            h.fragment_index = field(0xf8);
            h.skeleton_index = field(0xfc);
        }
        if be32(r0, 128) & 0x40 != 0 {
            h.exth = parse_exth(r0.get(16 + len..).unwrap_or(&[]), h.encoding);
        }
        Some(h)
    }

    pub fn is_utf8(&self) -> bool {
        self.encoding == 65001
    }
}

fn parse_exth(b: &[u8], encoding: u32) -> Exth {
    let mut e = Exth::default();
    if !b.starts_with(b"EXTH") {
        return e;
    }
    let count = be32(b, 8) as usize;
    let mut at = 12;
    for _ in 0..count.min(4096) {
        let kind = be32(b, at);
        let len = be32(b, at + 4) as usize;
        if len < 8 || at + len > b.len() {
            break;
        }
        let data = &b[at + 8..at + len];
        let text = || Some(decode(data, encoding)).filter(|s| !s.trim().is_empty());
        let num = || (data.len() >= 4).then(|| be32(data, 0));
        match kind {
            100 => e.authors.extend(text()),
            101 => e.publisher = text(),
            103 => e.description = text(),
            121 => e.kf8_boundary = num(),
            129 => e.cover_uri = text(),
            201 => e.cover_offset = num().filter(|&v| v != NONE),
            202 => e.thumb_offset = num().filter(|&v| v != NONE),
            503 => e.title = text(),
            524 => e.language = text(),
            _ => {}
        }
        at += len;
    }
    e
}

/// Text in the book's encoding: UTF-8, or Windows-1252.
pub(crate) fn decode(b: &[u8], encoding: u32) -> String {
    let s = if encoding == 65001 {
        String::from_utf8_lossy(b).into_owned()
    } else {
        akkhor_flow::encoding::decode_cp1252(b)
    };
    s.trim_end_matches('\0').trim().to_string()
}

/// A language from the MOBI header's locale field (a Windows language ID),
/// for the scripts Akkhor cares about most; others give nothing.
pub(crate) fn locale_language(locale: u32) -> Option<&'static str> {
    match locale & 0xff {
        0x09 => Some("en"),
        0x45 => Some("bn"),
        0x39 => Some("hi"),
        0x4e => Some("mr"),
        0x46 => Some("pa"),
        0x47 => Some("gu"),
        0x49 => Some("ta"),
        0x4a => Some("te"),
        0x4b => Some("kn"),
        0x4c => Some("ml"),
        0x20 => Some("ur"),
        0x01 => Some("ar"),
        _ => None,
    }
}
