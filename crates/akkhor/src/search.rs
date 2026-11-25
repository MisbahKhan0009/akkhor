//! Searching a whole book, a little at a time (Phase 5.3): the first hits
//! come at once, the rest as the app asks for them, and the app stops
//! whenever it likes by no longer asking.
//!
//! EPUBs and text files are searched chapter by chapter in their reflowed
//! text; PDFs page by page in their page text (hyphen-aware, Phase 4b), so
//! hits are page places, which the reflow view converts with
//! [`Reading::convert`](crate::Reading::convert). Indic spellings fold as
//! everywhere else (Phase 1).

use std::ops::Range;

use crate::reading::{Locator, Reading};
use crate::{Book, json};

/// One hit.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub start: Locator,
    pub end: Locator,
    /// The text around the hit, and where the hit is in it, in UTF-16
    /// units (what Kotlin and Java strings count).
    pub snippet: String,
    pub snippet_match: Range<usize>,
    /// Chapter (reflowed books) or page (PDFs).
    pub chapter_or_page: usize,
    /// The hit is in a picture's text (a table shown as a picture, a label
    /// on an illustration): show the picture, not a highlight.
    pub in_picture: bool,
}

impl Hit {
    pub fn to_json(&self) -> String {
        json::Obj::new()
            .str("start", &self.start.to_string())
            .str("end", &self.end.to_string())
            .str("snippet", &self.snippet)
            .num("match_start", self.snippet_match.start as f64)
            .num("match_end", self.snippet_match.end as f64)
            .num("at", self.chapter_or_page as f64)
            .bool("in_picture", self.in_picture)
            .end()
    }
}

/// Characters of context on each side of a hit.
const CONTEXT: usize = 40;

/// Chapters or pages looked at per call when nothing is found: keeps a
/// call short on a big book with few hits.
const STEP: usize = 24;

/// A search in progress.
pub struct BookSearch {
    query: String,
    /// Next chapter or page to search.
    next: usize,
    /// Hits found but not handed out yet.
    pending: Vec<Hit>,
}

/// What one call returns.
pub struct Batch {
    pub hits: Vec<Hit>,
    /// Nothing more to find.
    pub done: bool,
    /// Share of the book searched, 0 to 1.
    pub searched: f64,
}

impl Batch {
    pub fn to_json(&self) -> String {
        json::Obj::new()
            .raw("hits", &json::array(self.hits.iter().map(Hit::to_json)))
            .bool("done", self.done)
            .num("searched", self.searched)
            .end()
    }
}

impl BookSearch {
    pub fn new(query: &str) -> Self {
        Self {
            query: query.to_string(),
            next: 0,
            pending: Vec::new(),
        }
    }

    fn units(book: &Book, reading: &Reading) -> usize {
        match book {
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => p.page_count(),
            #[allow(unreachable_patterns)]
            _ => reading.chapter_count(),
        }
    }

    /// Up to `max` more hits, in book order.
    pub fn next(&mut self, book: &Book, reading: &mut Reading, max: usize) -> Batch {
        let total = Self::units(book, reading);
        let mut looked = 0;
        while self.pending.len() < max.max(1) && self.next < total && looked < STEP {
            let found = self.search_unit(book, reading, self.next);
            self.pending.extend(found);
            self.next += 1;
            looked += 1;
        }
        let n = self.pending.len().min(max.max(1));
        let hits: Vec<Hit> = self.pending.drain(..n).collect();
        Batch {
            done: self.next >= total && self.pending.is_empty(),
            searched: if total == 0 {
                1.0
            } else {
                self.next as f64 / total as f64
            },
            hits,
        }
    }

    fn search_unit(&self, book: &Book, reading: &mut Reading, unit: usize) -> Vec<Hit> {
        if self.query.trim().is_empty() {
            return Vec::new();
        }
        match book {
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => {
                let Ok(text) = p.page_text_cached(unit) else {
                    return Vec::new();
                };
                text.find(&self.query)
                    .into_iter()
                    .map(|r| {
                        let (snippet, m) = snippet_of(&text.text, r.clone());
                        Hit {
                            start: Locator::Page {
                                page: unit,
                                offset: r.start,
                            },
                            end: Locator::Page {
                                page: unit,
                                offset: r.end,
                            },
                            snippet,
                            snippet_match: m,
                            chapter_or_page: unit,
                            in_picture: false,
                        }
                    })
                    .collect()
            }
            #[allow(unreachable_patterns)]
            _ => {
                let Ok(c) = reading.chapter(book, unit) else {
                    return Vec::new();
                };
                crate::flow::search::find_all(&c.document, &self.query)
                    .into_iter()
                    .map(|m| {
                        let block = &c.document.blocks[m.start.block];
                        let end = if m.end.block == m.start.block {
                            m.end.offset
                        } else {
                            block.text.len()
                        };
                        let (snippet, sm) = snippet_of(&block.text, m.start.offset..end);
                        Hit {
                            start: Locator::Flow {
                                chapter: unit,
                                block: m.start.block,
                                offset: m.start.offset,
                            },
                            end: Locator::Flow {
                                chapter: unit,
                                block: m.end.block,
                                offset: m.end.offset,
                            },
                            snippet,
                            snippet_match: sm,
                            chapter_or_page: unit,
                            in_picture: block.kind == crate::flow::BlockKind::Image,
                        }
                    })
                    .collect()
            }
        }
    }
}

/// Up to [`CONTEXT`] characters each side of `range` in `text`, on one line,
/// and the range inside it in UTF-16 units.
pub fn snippet_of(text: &str, range: Range<usize>) -> (String, Range<usize>) {
    let from = text[..range.start]
        .char_indices()
        .rev()
        .nth(CONTEXT.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    let to = text[range.end..]
        .char_indices()
        .nth(CONTEXT)
        .map_or(text.len(), |(i, _)| range.end + i);
    let flat = |s: &str| s.replace(['\n', '\r', '\t'], " ");
    let before = flat(&text[from..range.start]);
    let hit = flat(&text[range.clone()]);
    let after = flat(&text[range.end..to]);
    let a = before.encode_utf16().count();
    let b = a + hit.encode_utf16().count();
    (format!("{before}{hit}{after}"), a..b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_count_utf16_and_stay_on_one_line() {
        let t = "এক\nদুই 😀 তিন চার";
        let i = t.find("তিন").unwrap();
        let (s, m) = snippet_of(t, i..i + "তিন".len());
        assert_eq!(s, "এক দুই 😀 তিন চার");
        let units: Vec<u16> = s.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[m]).unwrap(), "তিন");
    }
}
