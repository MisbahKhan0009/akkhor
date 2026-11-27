//! MOBI, AZW, AZW3 (KF8) and PalmDOC books for Akkhor.
//!
//! A Kindle book is a Palm database. Record 0 holds the headers and
//! metadata; the next records hold the book's text, compressed record by
//! record (PalmDOC LZ77 or HUFF/CDIC); then pictures, fonts and indexes.
//! Old books (MOBI 7, "KF7") are one HTML stream split into chapters at
//! `<mbp:pagebreak/>`. KF8 books (AZW3) rebuild each XHTML file from a
//! skeleton and fragments, with style sheets in separate flows. A
//! combined file holds both; the KF8 half is read.
//!
//! Opening reads only the headers and indexes, so a library card is
//! cheap; the text is decompressed when a chapter is first needed.

mod header;
mod huff;
mod index;
mod kf7;
mod lz77;
mod palmdb;

use std::sync::OnceLock;

use akkhor_core::{Error, Result, Source};
use akkhor_flow::Document;
use akkhor_flow::image::ImageProvider;
use akkhor_flow::model::linked_stylesheets;

use crate::header::{Header, NONE, decode, locale_language};
use crate::index::Index;
use crate::palmdb::PalmDb;

/// The most text a book may decompress to; a hostile file cannot ask for
/// more memory than this.
const MAX_TEXT: usize = 512 << 20;

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TocEntry {
    pub title: String,
    pub chapter: usize,
    pub depth: u8,
}

/// Which kind of book the file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// PalmDOC (`TEXtREAd`): plain text.
    PalmDoc,
    /// MOBI 7: one HTML stream.
    Kf7,
    /// KF8 (AZW3, or the KF8 half of a combined file).
    Kf8,
}

pub struct MobiBook {
    db: PalmDb,
    /// Record number of the section read (0, or the KF8 half's record 0).
    base: usize,
    header: Header,
    pub kind: Kind,
    pub metadata: Metadata,
    /// Absolute record number of the first picture.
    first_image: Option<usize>,
    text: OnceLock<std::result::Result<Text, String>>,
}

struct Text {
    chapters: Vec<String>,
    /// KF8 style sheets and other flows, by flow number.
    flows: Vec<String>,
    toc: Vec<TocEntry>,
}

impl MobiBook {
    pub fn open(source: Source) -> Result<Self> {
        source.advise_random();
        let db = PalmDb::open(source)?;
        let kind = match &db.kind {
            b"BOOKMOBI" => Kind::Kf7,
            b"TEXtREAd" => Kind::PalmDoc,
            _ => return Err(Error::malformed("not a MOBI or PalmDOC book")),
        };
        let first = Header::parse(db.record(0)).ok_or(Error::malformed("no MOBI header"))?;
        if kind == Kind::Kf7 && first.encryption != 0 {
            return Err(Error::Protected);
        }
        // Combined files: read the KF8 half when there is a sound one.
        let mut base = 0;
        let mut header = first.clone();
        let mut kind = kind;
        if first.version >= 8 {
            kind = Kind::Kf8;
        } else if let Some(b) = first.exth.kf8_boundary.filter(|&b| b != NONE && b > 0)
            && let Some(h) = Header::parse(db.record(b as usize)).filter(|h| h.version >= 8)
        {
            if h.encryption != 0 {
                return Err(Error::Protected);
            }
            base = b as usize;
            header = h;
            kind = Kind::Kf8;
        }
        // Metadata: the first header's EXTH is the complete one.
        let exth = if first.exth.title.is_some() || !first.exth.authors.is_empty() {
            &first.exth
        } else {
            &header.exth
        };
        let metadata = Metadata {
            title: exth.title.clone().or_else(|| first.full_name.clone()),
            authors: exth.authors.clone(),
            language: exth
                .language
                .clone()
                .or_else(|| locale_language(first.locale).map(str::to_string)),
            publisher: exth.publisher.clone(),
            description: exth.description.clone(),
        };
        // Pictures: a combined file keeps them once, in its first half,
        // where the first header points (absolute record numbers).
        let first_image = if base > 0 {
            Some(first.first_image)
        } else {
            Some(header.first_image)
        }
        .filter(|&v| v != NONE && v != 0)
        .map(|v| v as usize);
        Ok(MobiBook {
            db,
            base,
            header,
            kind,
            metadata,
            first_image,
            text: OnceLock::new(),
        })
    }

    fn text(&self) -> Result<&Text> {
        self.text
            .get_or_init(|| self.load().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::malformed(e.clone()))
    }

    pub fn chapter_count(&self) -> usize {
        self.text().map_or(0, |t| t.chapters.len())
    }

    pub fn toc(&self) -> Vec<TocEntry> {
        self.text().map(|t| t.toc.clone()).unwrap_or_default()
    }

    /// One chapter as a document.
    pub fn chapter(&self, index: usize) -> Result<Document> {
        let t = self.text()?;
        let html = t.chapters.get(index).ok_or(Error::OutOfRange)?;
        let mut doc = match self.kind {
            Kind::PalmDoc => Document::from_plain_text(html),
            Kind::Kf7 => Document::from_html(&kf7::normalize(html)),
            Kind::Kf8 => {
                let css: Vec<&str> = linked_stylesheets(html)
                    .iter()
                    .filter_map(|href| flow_number(href).and_then(|n| t.flows.get(n)))
                    .map(String::as_str)
                    .collect();
                Document::from_html_with_css(html, &css)
            }
        };
        if doc.lang.is_none() {
            doc.lang = self.metadata.language.clone();
        }
        Ok(doc)
    }

    /// Size of each chapter's text, for progress through the book.
    pub fn chapter_sizes(&self) -> Vec<u64> {
        self.text()
            .map(|t| t.chapters.iter().map(|c| c.len().max(1) as u64).collect())
            .unwrap_or_default()
    }

    /// Pictures for layout and painting (the same for every chapter).
    pub fn chapter_images(&self, _index: usize) -> Images<'_> {
        Images { book: self }
    }

    /// The cover picture's bytes, if the book names one.
    pub fn cover_bytes(&self) -> Option<Vec<u8>> {
        let exths = [&self.header.exth, &Header::parse(self.db.record(0))?.exth];
        let offset = exths
            .iter()
            .find_map(|e| e.cover_offset.or(e.thumb_offset))?;
        let bytes = self.resource(offset as usize + 1)?;
        is_picture(bytes).then(|| bytes.to_vec())
    }

    /// Resource `n` (1-based, as `recindex` and `kindle:embed:` count).
    fn resource(&self, n: usize) -> Option<&[u8]> {
        let first = self.first_image?;
        let rec = first.checked_add(n.checked_sub(1)?)?;
        (rec < self.db.len()).then(|| self.db.record(rec))
    }

    /// Decompresses the text and splits it into chapters.
    fn load(&self) -> Result<Text> {
        let raw = self.raw_text()?;
        match self.kind {
            Kind::PalmDoc => Ok(Text {
                chapters: vec![akkhor_flow::encoding::decode_text(&raw).text],
                flows: Vec::new(),
                toc: Vec::new(),
            }),
            Kind::Kf7 => Ok(self.kf7(&raw)),
            Kind::Kf8 => self.kf8(&raw),
        }
    }

    fn raw_text(&self) -> Result<Vec<u8>> {
        let h = &self.header;
        let mut huff = None;
        if h.compression == 17480 {
            let first = self.base + h.huff_record as usize;
            let cdics: Vec<&[u8]> = (1..h.huff_count.min(256) as usize)
                .map(|k| self.db.record(first + k))
                .collect();
            huff = Some(huff::Huff::new(self.db.record(first), &cdics)?);
        }
        let mut out = Vec::with_capacity((h.text_length as usize).min(64 << 20));
        for i in 1..=h.text_records as usize {
            let rec = strip_trailers(self.db.record(self.base + i), h.extra_flags);
            match h.compression {
                1 => out.extend_from_slice(rec),
                2 => lz77::decompress(rec, &mut out),
                17480 => huff
                    .as_mut()
                    .expect("set above")
                    .decompress(rec, &mut out)?,
                c => return Err(Error::malformed(format!("unknown compression {c}"))),
            }
            if out.len() > MAX_TEXT {
                return Err(Error::malformed("text too large"));
            }
        }
        Ok(out)
    }

    /// MOBI 7: one HTML stream, chapters at page breaks; the `<head>`
    /// (style) goes with every chapter.
    fn kf7(&self, raw: &[u8]) -> Text {
        let head_end = find_ci(raw, b"</head>", 0).map_or(0, |i| i + 7);
        let head = &raw[..head_end];
        let mut cuts = vec![head_end];
        let mut at = head_end;
        while let Some(i) = find_ci(raw, b"<mbp:pagebreak", at) {
            cuts.push(i);
            at = i + 1;
        }
        cuts.push(raw.len());
        let mut chapters = Vec::new();
        let mut starts = Vec::new();
        for w in cuts.windows(2) {
            let body = &raw[w[0]..w[1]];
            if !has_text(body) && !find_ci(body, b"<img", 0).is_some() {
                continue;
            }
            let mut html = head.to_vec();
            html.extend_from_slice(body);
            chapters.push(self.html_string(&rewrite_recindex(&html)));
            starts.push(w[0]);
        }
        if chapters.is_empty() {
            chapters.push(self.html_string(&rewrite_recindex(raw)));
            starts.push(0);
        }
        let toc = Index::read(&self.db, self.base, self.header.ncx_index)
            .map(|idx| {
                idx.entries
                    .iter()
                    .filter_map(|e| {
                        let pos = e.tag(1, 0)? as usize;
                        let title = idx.string(e.tag(3, 0)?)?;
                        Some(TocEntry {
                            title: decode(title, self.header.encoding),
                            chapter: starts.iter().rposition(|&s| s <= pos).unwrap_or(0),
                            depth: e.tag(4, 0).unwrap_or(0).min(16) as u8,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Text {
            chapters,
            flows: Vec::new(),
            toc,
        }
    }

    /// KF8: flows from the FDST table; each XHTML file rebuilt from its
    /// skeleton and fragments.
    fn kf8(&self, raw: &[u8]) -> Result<Text> {
        let h = &self.header;
        let mut bounds = vec![0usize];
        let fdst = self.db.record(self.base + h.fdst as usize);
        if h.fdst != NONE && fdst.starts_with(b"FDST") {
            let n = palmdb::be32(fdst, 8) as usize;
            bounds = (0..n.min(4096))
                .map(|k| palmdb::be32(fdst, 12 + k * 8) as usize)
                .collect();
            if bounds.is_empty() {
                bounds.push(0);
            }
        }
        bounds.push(raw.len());
        let flow = |k: usize| -> &[u8] {
            let s = bounds[k].min(raw.len());
            let e = bounds[k + 1].clamp(s, raw.len());
            &raw[s..e]
        };
        let text = flow(0);
        let flows: Vec<String> = (0..bounds.len() - 1)
            .map(|k| self.html_string(flow(k)))
            .collect();

        let skel = Index::read(&self.db, self.base, h.skeleton_index)
            .ok_or(Error::malformed("no skeleton index"))?;
        let frag = Index::read(&self.db, self.base, h.fragment_index)
            .ok_or(Error::malformed("no fragment index"))?;
        let frags: Vec<(usize, usize, usize)> = frag
            .entries
            .iter()
            .map(|e| {
                let insert = std::str::from_utf8(&e.label)
                    .ok()
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
                (
                    insert,
                    e.tag(3, 0).unwrap_or(0) as usize,
                    e.tag(6, 1).unwrap_or(0) as usize,
                )
            })
            .collect();
        let mut chapters = Vec::new();
        let mut next = 0;
        for e in &skel.entries {
            let count = e.tag(1, 0).unwrap_or(0) as usize;
            let start = e.tag(6, 0).unwrap_or(0) as usize;
            let len = e.tag(6, 1).unwrap_or(0) as usize;
            let s = start.min(text.len());
            let mut base = (start + len).min(text.len());
            let mut file = text[s..base].to_vec();
            for &(insert, _, flen) in frags.iter().skip(next).take(count) {
                let piece = &text[base.min(text.len())..(base + flen).min(text.len())];
                let at = insert.saturating_sub(start).min(file.len());
                file.splice(at..at, piece.iter().copied());
                base += flen;
            }
            next += count;
            chapters.push(self.html_string(&file));
        }
        let toc = Index::read(&self.db, self.base, h.ncx_index)
            .map(|idx| {
                idx.entries
                    .iter()
                    .filter_map(|e| {
                        let title = idx.string(e.tag(3, 0)?)?;
                        let chapter = e
                            .tag(6, 0)
                            .and_then(|fid| frags.get(fid as usize))
                            .map_or(0, |f| f.1);
                        Some(TocEntry {
                            title: decode(title, h.encoding),
                            chapter: chapter.min(chapters.len().saturating_sub(1)),
                            depth: e.tag(4, 0).unwrap_or(0).min(16) as u8,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Text {
            chapters,
            flows,
            toc,
        })
    }

    fn html_string(&self, b: &[u8]) -> String {
        if self.header.is_utf8() {
            String::from_utf8_lossy(b).into_owned()
        } else {
            akkhor_flow::encoding::decode_cp1252(b)
        }
    }
}

/// Pictures of a MOBI book: `recindex:N` (MOBI 7) and `kindle:embed:XXXX`
/// (KF8) sources.
pub struct Images<'a> {
    book: &'a MobiBook,
}

impl ImageProvider for Images<'_> {
    fn bytes(&self, src: &str) -> Option<Vec<u8>> {
        let n = if let Some(n) = src.strip_prefix("recindex:") {
            n.trim().parse::<usize>().ok()?
        } else {
            base32(src.strip_prefix("kindle:embed:")?.split('?').next()?)?
        };
        let bytes = self.book.resource(n)?;
        is_picture(bytes).then(|| bytes.to_vec())
    }
}

/// Removes the trailing entries the extra-data flags announce.
fn strip_trailers(mut rec: &[u8], flags: u16) -> &[u8] {
    for bit in 1..16 {
        if flags & (1 << bit) != 0 {
            let mut n = 0usize;
            for &v in &rec[rec.len().saturating_sub(4)..] {
                if v & 0x80 != 0 {
                    n = 0;
                }
                n = (n << 7) | usize::from(v & 0x7f);
            }
            rec = &rec[..rec.len().saturating_sub(n)];
        }
    }
    if flags & 1 != 0
        && let Some(&last) = rec.last()
    {
        rec = &rec[..rec.len().saturating_sub(usize::from(last & 3) + 1)];
    }
    rec
}

/// `recindex="00001"` (an attribute MOBI 7 uses for pictures) becomes a
/// `src` the image provider understands.
fn rewrite_recindex(html: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(html.len());
    let mut i = 0;
    while i < html.len() {
        if html[i..].starts_with(b" recindex=\"") || html[i..].starts_with(b" RECINDEX=\"") {
            out.extend_from_slice(b" src=\"recindex:");
            i += 11;
        } else {
            out.push(html[i]);
            i += 1;
        }
    }
    out
}

/// KF8 numbers are base 32 with digits 0-9 and A-V.
fn base32(s: &str) -> Option<usize> {
    let mut v: usize = 0;
    for c in s.chars() {
        v = v.checked_mul(32)?.checked_add(c.to_digit(32)? as usize)?;
    }
    Some(v)
}

/// `kindle:flow:0001?mime=text/css` → flow 1.
fn flow_number(href: &str) -> Option<usize> {
    base32(href.strip_prefix("kindle:flow:")?.split('?').next()?)
}

fn is_picture(b: &[u8]) -> bool {
    b.starts_with(&[0xff, 0xd8])
        || b.starts_with(b"\x89PNG")
        || b.starts_with(b"GIF8")
        || b.starts_with(b"BM")
}

fn find_ci(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
        .map(|p| p + from)
}

/// Whether HTML has any text outside its tags.
fn has_text(html: &[u8]) -> bool {
    let mut in_tag = false;
    for &b in html {
        match b {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            _ if !in_tag && !b.is_ascii_whitespace() => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_and_flows() {
        assert_eq!(base32("0001"), Some(1));
        assert_eq!(base32("000V"), Some(31));
        assert_eq!(base32("0010"), Some(32));
        assert_eq!(flow_number("kindle:flow:0002?mime=text/css"), Some(2));
    }

    #[test]
    fn trailing_entries_are_removed() {
        // A multibyte count byte (0: one byte), then a 3-byte trailing
        // entry whose size (0x83) is its last byte.
        let rec = b"text\x00\xaa\xbb\x83";
        assert_eq!(strip_trailers(rec, 0b11), b"text");
    }
}
