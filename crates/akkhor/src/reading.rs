//! Reading a book as the app does: places that survive font size, screen and
//! theme changes (locators), progress through the whole book, and the
//! book's reflowed chapters, one kept at a time.
//!
//! A **locator** is the one value the app stores for a place:
//!
//! - `akkhor:1/c<chapter>/b<block>/o<byte>` in reflowed text (EPUB, a text
//!   file, a PDF's reflow), a byte offset into one block of one chapter;
//! - `akkhor:1/p<page>/o<byte>` on a PDF page, a byte offset into that
//!   page's text.
//!
//! Neither depends on layout, so a place stays put when the font size, the
//! screen or the theme changes. A locator pointing past the end of what it
//! names opens at the end; a PDF's two kinds convert into each other.

use std::fmt;
#[cfg(feature = "pdf")]
use std::ops::Range;

use crate::flow::Document;
use crate::{Book, Error, Result};

/// A place in a book; see the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Locator {
    /// Reflowed text: chapter, block, byte offset in the block's text.
    Flow {
        chapter: usize,
        block: usize,
        offset: usize,
    },
    /// A PDF page: page, byte offset in the page's text.
    Page { page: usize, offset: usize },
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Locator::Flow {
                chapter,
                block,
                offset,
            } => write!(f, "akkhor:1/c{chapter}/b{block}/o{offset}"),
            Locator::Page { page, offset } => write!(f, "akkhor:1/p{page}/o{offset}"),
        }
    }
}

impl Locator {
    /// The start of the book (its first chapter or page).
    pub const START: Locator = Locator::Flow {
        chapter: 0,
        block: 0,
        offset: 0,
    };

    /// Reads a locator written by [`Display`](fmt::Display). Unknown parts
    /// are ignored (a later version may add some); a missing offset is 0.
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.trim().strip_prefix("akkhor:")?;
        let mut parts = rest.split('/');
        let version: u32 = parts.next()?.parse().ok()?;
        if version != 1 {
            return None;
        }
        let (mut chapter, mut block, mut page, mut offset) = (None, None, None, None);
        for p in parts {
            let (key, value) = p.split_at(p.char_indices().nth(1).map_or(p.len(), |(i, _)| i));
            let Ok(v) = value.parse::<usize>() else {
                continue;
            };
            match key {
                "c" => chapter = Some(v),
                "b" => block = Some(v),
                "p" => page = Some(v),
                "o" => offset = Some(v),
                _ => {}
            }
        }
        let offset = offset.unwrap_or(0);
        match (page, chapter) {
            (Some(page), _) => Some(Locator::Page { page, offset }),
            (None, Some(chapter)) => Some(Locator::Flow {
                chapter,
                block: block.unwrap_or(0),
                offset,
            }),
            (None, None) => None,
        }
    }
}

/// One reflowed chapter.
pub struct Chapter {
    pub index: usize,
    pub document: Document,
    /// For a PDF: the map between this chapter and its pages.
    #[cfg(feature = "pdf")]
    pub reflow: Option<crate::pdf::reflow::Reflow>,
}

impl Chapter {
    /// Byte position of (block, offset) counted from the chapter's start.
    pub fn position(&self, block: usize, offset: usize) -> usize {
        let before: usize = self
            .document
            .blocks
            .iter()
            .take(block)
            .map(|b| b.text.len())
            .sum();
        before + offset.min(self.document.blocks.get(block).map_or(0, |b| b.text.len()))
    }

    pub fn len(&self) -> usize {
        self.document.blocks.iter().map(|b| b.text.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The (block, offset) at byte `pos` from the chapter's start, on a
    /// character boundary.
    pub fn at(&self, pos: usize) -> (usize, usize) {
        let mut left = pos;
        for (i, b) in self.document.blocks.iter().enumerate() {
            if left < b.text.len() || i + 1 == self.document.blocks.len() {
                return (i, floor_char(&b.text, left.min(b.text.len())));
            }
            left -= b.text.len();
        }
        (0, 0)
    }

    /// The end of the chapter.
    pub fn end(&self) -> (usize, usize) {
        let n = self.document.blocks.len();
        match n {
            0 => (0, 0),
            _ => (n - 1, self.document.blocks[n - 1].text.len()),
        }
    }
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Reading state for one open book: its chapters and how much text each
/// holds, and the last chapter used.
pub struct Reading {
    /// PDF pages of each reflow chapter (empty for other formats).
    #[cfg(feature = "pdf")]
    pdf_chapters: Vec<Range<usize>>,
    /// A measure of each chapter's length, for progress.
    weights: Vec<u64>,
    current: Option<Chapter>,
}

impl Reading {
    pub fn new(book: &Book) -> Self {
        #[cfg(feature = "pdf")]
        let pdf_chapters = match book {
            Book::Pdf(p) => p.reflow_chapters(),
            #[allow(unreachable_patterns)]
            _ => Vec::new(),
        };
        let weights = match book {
            #[cfg(feature = "pdf")]
            Book::Pdf(_) => pdf_chapters.iter().map(|r| r.len() as u64).collect(),
            #[cfg(feature = "epub")]
            Book::Epub(e) => e.chapter_sizes(),
            #[cfg(feature = "txt")]
            Book::Text(_) => vec![1],
        };
        Self {
            #[cfg(feature = "pdf")]
            pdf_chapters,
            weights,
            current: None,
        }
    }

    /// Reflowed chapters: EPUB chapters, one for a text file, a PDF's
    /// reflow chapters.
    pub fn chapter_count(&self) -> usize {
        self.weights.len()
    }

    /// The PDF pages of reflow chapter `i`.
    #[cfg(feature = "pdf")]
    pub fn chapter_pages(&self, i: usize) -> Option<Range<usize>> {
        self.pdf_chapters.get(i).cloned()
    }

    /// The reflow chapter holding PDF page `page`.
    #[cfg(feature = "pdf")]
    pub fn chapter_of_page(&self, page: usize) -> Option<usize> {
        self.pdf_chapters.iter().position(|r| r.contains(&page))
    }

    /// Chapter `i`, read (or reflowed) if it is not the one kept.
    pub fn chapter(&mut self, book: &Book, i: usize) -> Result<&Chapter> {
        if self.current.as_ref().is_none_or(|c| c.index != i) {
            if i >= self.chapter_count() {
                return Err(Error::OutOfRange);
            }
            self.current = Some(self.load(book, i)?);
        }
        Ok(self.current.as_ref().expect("just loaded"))
    }

    fn load(&self, book: &Book, i: usize) -> Result<Chapter> {
        Ok(match book {
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => {
                let range = self.pdf_chapters.get(i).cloned().ok_or(Error::OutOfRange)?;
                let r = p.reflow(range)?;
                Chapter {
                    index: i,
                    document: r.document.clone(),
                    reflow: Some(r),
                }
            }
            #[cfg(feature = "epub")]
            Book::Epub(e) => Chapter {
                index: i,
                document: e.chapter(i)?,
                #[cfg(feature = "pdf")]
                reflow: None,
            },
            #[cfg(feature = "txt")]
            Book::Text(t) => Chapter {
                index: i,
                document: t.document.clone(),
                #[cfg(feature = "pdf")]
                reflow: None,
            },
        })
    }

    /// The same place, inside what it names: a chapter, block, page or
    /// offset past the end becomes the end; offsets land on characters.
    pub fn clamp(&mut self, book: &Book, loc: Locator) -> Result<Locator> {
        match loc {
            Locator::Flow {
                chapter,
                block,
                offset,
            } => {
                let n = self.chapter_count();
                if n == 0 {
                    return Ok(Locator::START);
                }
                let past = chapter >= n;
                let chapter = chapter.min(n - 1);
                let c = self.chapter(book, chapter)?;
                let (block, offset) = if past || block >= c.document.blocks.len() {
                    c.end()
                } else {
                    let t = &c.document.blocks[block].text;
                    (block, floor_char(t, offset.min(t.len())))
                };
                Ok(Locator::Flow {
                    chapter,
                    block,
                    offset,
                })
            }
            Locator::Page { page, offset } => {
                #[cfg(feature = "pdf")]
                #[allow(irrefutable_let_patterns)]
                if let Book::Pdf(p) = book {
                    let n = p.page_count();
                    if n == 0 {
                        return Ok(Locator::Page { page: 0, offset: 0 });
                    }
                    let past = page >= n;
                    let page = page.min(n - 1);
                    let text = p.page_text_cached(page)?;
                    let offset = if past {
                        text.text.len()
                    } else {
                        floor_char(&text.text, offset.min(text.text.len()))
                    };
                    return Ok(Locator::Page { page, offset });
                }
                let _ = (page, offset);
                Err(Error::OutOfRange)
            }
        }
    }

    /// How far through the book `loc` is, 0 to 1. Chapters count by their
    /// size (an EPUB chapter's file size, a PDF chapter's pages), places
    /// inside a chapter by text.
    pub fn progress(&mut self, book: &Book, loc: Locator) -> Result<f64> {
        let loc = self.clamp(book, loc)?;
        match loc {
            Locator::Flow {
                chapter,
                block,
                offset,
            } => {
                let total: u64 = self.weights.iter().sum::<u64>().max(1);
                let before: u64 = self.weights.iter().take(chapter).sum();
                let weight = self.weights.get(chapter).copied().unwrap_or(0);
                let c = self.chapter(book, chapter)?;
                let len = c.len().max(1);
                let within = c.position(block, offset) as f64 / len as f64;
                Ok(((before as f64 + weight as f64 * within) / total as f64).clamp(0.0, 1.0))
            }
            Locator::Page { page, offset } => {
                #[cfg(feature = "pdf")]
                #[allow(irrefutable_let_patterns)]
                if let Book::Pdf(p) = book {
                    let n = p.page_count().max(1);
                    let len = p.page_text_cached(page)?.text.len().max(1);
                    let within = offset as f64 / len as f64;
                    return Ok(((page as f64 + within) / n as f64).clamp(0.0, 1.0));
                }
                let _ = (page, offset);
                Err(Error::OutOfRange)
            }
        }
    }

    /// The place at `progress` (0 to 1) through the reflowed book, for a
    /// scrubber. The inverse of [`progress`](Self::progress), to the byte.
    pub fn at_progress(&mut self, book: &Book, progress: f64) -> Result<Locator> {
        let n = self.chapter_count();
        if n == 0 {
            return Ok(Locator::START);
        }
        let total: u64 = self.weights.iter().sum::<u64>().max(1);
        let target = progress.clamp(0.0, 1.0) * total as f64;
        let mut before = 0u64;
        let mut chapter = n - 1;
        for (i, w) in self.weights.iter().enumerate() {
            if (before + w) as f64 > target || i + 1 == n {
                chapter = i;
                break;
            }
            before += w;
        }
        let weight = self.weights[chapter].max(1) as f64;
        let within = ((target - before as f64) / weight).clamp(0.0, 1.0);
        let c = self.chapter(book, chapter)?;
        let (block, offset) = c.at((within * c.len() as f64).round() as usize);
        Ok(Locator::Flow {
            chapter,
            block,
            offset,
        })
    }

    /// A PDF locator in the other view: a page place as a reflow place, a
    /// reflow place as a page place. Other books return `loc` unchanged.
    pub fn convert(&mut self, book: &Book, loc: Locator) -> Result<Locator> {
        #[cfg(feature = "pdf")]
        if matches!(book, Book::Pdf(_)) {
            let loc = self.clamp(book, loc)?;
            return match loc {
                Locator::Page { page, offset } => {
                    let chapter = self.chapter_of_page(page).ok_or(Error::OutOfRange)?;
                    let c = self.chapter(book, chapter)?;
                    let r = c.reflow.as_ref().ok_or(Error::OutOfRange)?;
                    let (block, offset) = r.from_page(page, offset).unwrap_or((0, 0));
                    Ok(Locator::Flow {
                        chapter,
                        block,
                        offset,
                    })
                }
                Locator::Flow {
                    chapter,
                    block,
                    offset,
                } => {
                    let c = self.chapter(book, chapter)?;
                    let r = c.reflow.as_ref().ok_or(Error::OutOfRange)?;
                    let (page, offset) = r
                        .to_page(block, offset)
                        .or_else(|| r.pages.clone().next().map(|p| (p, 0)))
                        .ok_or(Error::OutOfRange)?;
                    Ok(Locator::Page { page, offset })
                }
            };
        }
        let _ = book;
        Ok(loc)
    }
}

/// A sentence for read aloud.
#[derive(Clone, Debug, PartialEq)]
pub struct Sentence {
    pub start: Locator,
    pub end: Locator,
    pub text: String,
}

impl Sentence {
    pub fn to_json(&self) -> String {
        crate::json::Obj::new()
            .str("start", &self.start.to_string())
            .str("end", &self.end.to_string())
            .str("text", &self.text)
            .end()
    }
}

/// Most text handed out by one [`Reading::text_between`] call.
pub const MAX_TEXT: usize = 1 << 20;

/// Words ending in a full stop that do not end a sentence.
const ABBREVIATIONS: &[&str] = &[
    "Mr", "Mrs", "Ms", "Dr", "Prof", "Sr", "Jr", "St", "Mt", "vs", "etc", "e.g", "i.e", "No",
    "Fig", "pp", "Vol", "Ch", "approx", "Inc", "Ltd", "Co",
];

/// Byte ranges of the sentences of `text`: ending at `.`, `?`, `!`, `।`,
/// `॥` (with closing quotes and brackets after them) followed by a space,
/// or at the end. A full stop after a known abbreviation or a single
/// capital letter (an initial) does not end one.
pub fn sentence_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        if matches!(c, '.' | '?' | '!' | '।' | '॥') {
            // Take the run of marks and closing quotes after it.
            let mut j = i + 1;
            while j < chars.len()
                && matches!(
                    chars[j].1,
                    '.' | '?' | '!' | '।' | '॥' | '"' | '\'' | '”' | '’' | ')' | ']' | '»'
                )
            {
                j += 1;
            }
            let end = chars.get(j).map_or(text.len(), |(b, _)| *b);
            let next_space = chars.get(j).is_none_or(|(_, n)| n.is_whitespace());
            let abbreviation = c == '.' && {
                let word = text[start..at]
                    .rsplit(|ch: char| ch.is_whitespace() || ch == '(')
                    .next()
                    .unwrap_or("");
                ABBREVIATIONS.contains(&word)
                    || (word.chars().count() == 1 && word.chars().all(char::is_uppercase))
            };
            if next_space && !abbreviation {
                if !text[start..end].trim().is_empty() {
                    out.push(start..end);
                }
                // The next sentence starts after the spaces.
                let mut k = j;
                while k < chars.len() && chars[k].1.is_whitespace() {
                    k += 1;
                }
                start = chars.get(k).map_or(text.len(), |(b, _)| *b);
                i = k;
                continue;
            }
            i = j;
            continue;
        }
        let _ = at;
        i += 1;
    }
    if !text[start..].trim().is_empty() {
        out.push(start..text.trim_end().len().max(start));
    }
    out
}

impl Reading {
    /// The text from `start` to `end`, across blocks, chapters or pages
    /// (blocks joined by a line break), at most [`MAX_TEXT`] bytes. Both
    /// must be of one kind (two reflow places, or two page places).
    pub fn text_between(&mut self, book: &Book, start: Locator, end: Locator) -> Result<String> {
        let (start, end) = (self.clamp(book, start)?, self.clamp(book, end)?);
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        let mut out = String::new();
        match (start, end) {
            (
                Locator::Flow {
                    chapter: c0,
                    block: b0,
                    offset: o0,
                },
                Locator::Flow {
                    chapter: c1,
                    block: b1,
                    offset: o1,
                },
            ) => {
                for chapter in c0..=c1 {
                    let c = self.chapter(book, chapter)?;
                    for (bi, block) in c.document.blocks.iter().enumerate() {
                        let from = if (chapter, bi) == (c0, b0) {
                            o0
                        } else if (chapter, bi) < (c0, b0) {
                            continue;
                        } else {
                            0
                        };
                        let to = if (chapter, bi) == (c1, b1) {
                            o1
                        } else if (chapter, bi) > (c1, b1) {
                            break;
                        } else {
                            block.text.len()
                        };
                        if block.kind == crate::flow::BlockKind::Image || from >= to {
                            continue;
                        }
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&block.text[from..to]);
                        if out.len() >= MAX_TEXT {
                            out.truncate(floor_char(&out, MAX_TEXT));
                            return Ok(out);
                        }
                    }
                }
                Ok(out)
            }
            (
                Locator::Page {
                    page: p0,
                    offset: o0,
                },
                Locator::Page {
                    page: p1,
                    offset: o1,
                },
            ) => {
                #[cfg(feature = "pdf")]
                #[allow(irrefutable_let_patterns)]
                if let Book::Pdf(p) = book {
                    for page in p0..=p1 {
                        let t = p.page_text_cached(page)?;
                        let from = if page == p0 { o0 } else { 0 };
                        let to = if page == p1 { o1 } else { t.text.len() };
                        if from < to {
                            if !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str(&t.text[from..to]);
                        }
                        if out.len() >= MAX_TEXT {
                            out.truncate(floor_char(&out, MAX_TEXT));
                            break;
                        }
                    }
                    return Ok(out);
                }
                let _ = (p0, o0, p1, o1);
                Err(Error::OutOfRange)
            }
            _ => Err(Error::OutOfRange),
        }
    }

    /// Up to `count` sentences from `from` on, for read aloud: in reflowed
    /// text (a PDF page place is converted), skipping pictures; a sentence
    /// never crosses a block, so a heading is read on its own.
    pub fn sentences(&mut self, book: &Book, from: Locator, count: usize) -> Result<Vec<Sentence>> {
        let from = match self.clamp(book, from)? {
            l @ Locator::Page { .. } => self.convert(book, l)?,
            l => l,
        };
        let Locator::Flow {
            chapter,
            block,
            offset,
        } = from
        else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for ch in chapter..self.chapter_count() {
            let c = self.chapter(book, ch)?;
            for (bi, b) in c.document.blocks.iter().enumerate() {
                if (ch, bi) < (chapter, block) || b.kind == crate::flow::BlockKind::Image {
                    continue;
                }
                for r in sentence_ranges(&b.text) {
                    // From the sentence holding `from`.
                    if (ch, bi) == (chapter, block) && r.end <= offset {
                        continue;
                    }
                    out.push(Sentence {
                        start: Locator::Flow {
                            chapter: ch,
                            block: bi,
                            offset: r.start,
                        },
                        end: Locator::Flow {
                            chapter: ch,
                            block: bi,
                            offset: r.end,
                        },
                        text: b.text[r].to_string(),
                    });
                    if out.len() >= count {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// The screen (laid-out page) of a chapter that shows (block, offset): the
/// last one starting at or before it.
pub fn screen_of(pages: &[crate::flow::Page], block: usize, offset: usize) -> usize {
    let mut found = 0;
    for (i, p) in pages.iter().enumerate() {
        match p.start() {
            Some(start) if start <= (block, offset) => found = i,
            Some(_) => break,
            None => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_end_at_indic_and_latin_marks() {
        let split = |t: &str| -> Vec<String> {
            sentence_ranges(t)
                .into_iter()
                .map(|r| t[r].to_string())
                .collect()
        };
        assert_eq!(
            split("ভোর হলো। পাখি ডাকে! তুমি কি শুনছ?"),
            ["ভোর হলো।", "পাখি ডাকে!", "তুমি কি শুনছ?"]
        );
        assert_eq!(
            split("राम आया। सीता गई॥ अंत"),
            ["राम आया।", "सीता गई॥", "अंत"]
        );
        assert_eq!(
            split("Dr. Rahman met J. Smith at 3.5 km. \"Hello.\" Done"),
            ["Dr. Rahman met J. Smith at 3.5 km.", "\"Hello.\"", "Done"]
        );
        assert!(split("   ").is_empty());
    }

    #[test]
    fn locators_round_trip() {
        for l in [
            Locator::Flow {
                chapter: 3,
                block: 12,
                offset: 340,
            },
            Locator::Page { page: 7, offset: 0 },
        ] {
            assert_eq!(Locator::parse(&l.to_string()), Some(l));
        }
        assert_eq!(
            Locator::parse("akkhor:1/c2/b5"),
            Some(Locator::Flow {
                chapter: 2,
                block: 5,
                offset: 0
            })
        );
        // A later version's extra part is ignored; another version is not read.
        assert!(Locator::parse("akkhor:1/c2/b5/o9/x77").is_some());
        assert_eq!(Locator::parse("akkhor:2/c2"), None);
        assert_eq!(Locator::parse("garbage"), None);
        assert_eq!(Locator::parse("akkhor:1/"), None);
    }
}
