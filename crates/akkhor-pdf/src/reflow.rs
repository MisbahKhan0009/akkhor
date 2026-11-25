//! Reflow: a chapter of PDF pages as a [`Document`] that the flow layout
//! (the EPUB pipeline) can set for any screen, with a two-way map between
//! positions in the reflowed text and positions in the page text.
//!
//! Built from [`structure`](crate::structure): furniture is left out, a
//! paragraph that runs on into the next column or page is joined, line-end
//! hyphens are removed (unless the hyphenated word also appears inside a
//! line: a real compound such as পাঠ-ইঞ্জিন), footnotes follow their page's
//! text in a smaller size, and pictures become image blocks.

use std::collections::HashSet;
use std::ops::Range;

use akkhor_flow::model::{Block, BlockKind as FlowKind, BlockStyle, Document, Style, StyleSpan};
use hayro::kurbo::Rect;

use crate::structure::{self, BlockKind, PageInput};
use crate::text::PageText;

/// A reflowed chapter.
#[derive(Clone, Debug, Default)]
pub struct Reflow {
    pub document: Document,
    /// The PDF pages the chapter covers.
    pub pages: Range<usize>,
    /// For each document block, where its text comes from.
    segments: Vec<Vec<Segment>>,
}

/// A run of a block's text copied from one line of one page.
#[derive(Clone, Debug, PartialEq)]
struct Segment {
    /// Byte offset in the block's text.
    at: usize,
    /// PDF page and byte range in that page's text.
    page: usize,
    range: Range<usize>,
}

/// Image blocks name their source as `pdf:PAGE:X0,Y0,X1,Y1` (points).
pub fn image_source(page: usize, r: Rect) -> String {
    format!("pdf:{page}:{:.1},{:.1},{:.1},{:.1}", r.x0, r.y0, r.x1, r.y1)
}

/// Parses an image source written by [`image_source`].
pub fn parse_image_source(s: &str) -> Option<(usize, Rect)> {
    let rest = s.strip_prefix("pdf:")?;
    let (page, coords) = rest.split_once(':')?;
    let v: Vec<f64> = coords
        .split(',')
        .map(|c| c.parse().ok())
        .collect::<Option<_>>()?;
    let [x0, y0, x1, y1] = v[..] else { return None };
    Some((page.parse().ok()?, Rect::new(x0, y0, x1, y1)))
}

impl Reflow {
    /// The page position (page, byte offset in its text) of a reflow
    /// position (block, byte offset in the block's text).
    pub fn to_page(&self, block: usize, offset: usize) -> Option<(usize, usize)> {
        let segs = self.segments.get(block)?;
        let s = segs
            .iter()
            .rev()
            .find(|s| s.at <= offset)
            .or(segs.first())?;
        let off = s.range.start + offset.saturating_sub(s.at);
        Some((s.page, off.min(s.range.end)))
    }

    /// The reflow position of a page position: the block and offset whose
    /// text came from there, or the nearest one before it on that page.
    pub fn from_page(&self, page: usize, offset: usize) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, usize)> = None; // (block, offset, distance)
        for (b, segs) in self.segments.iter().enumerate() {
            for s in segs.iter().filter(|s| s.page == page) {
                if s.range.contains(&offset) || s.range.end == offset {
                    return Some((b, s.at + offset - s.range.start));
                }
                if s.range.end <= offset {
                    let d = offset - s.range.end;
                    if best.is_none_or(|(_, _, bd)| d < bd) {
                        best = Some((b, s.at + s.range.len(), d));
                    }
                }
            }
        }
        best.map(|(b, o, _)| (b, o)).or_else(|| {
            // Nothing on the page before it (furniture, a picture page):
            // the first block that starts on this page or later.
            self.segments
                .iter()
                .position(|segs| segs.iter().any(|s| s.page >= page))
                .map(|b| (b, 0))
        })
    }

    /// The page a block starts on.
    pub fn block_page(&self, block: usize) -> Option<usize> {
        self.segments.get(block)?.first().map(|s| s.page)
    }
}

fn ends_sentence(s: &str) -> bool {
    let t = s.trim_end();
    t.ends_with(['.', '।', '॥', '?', '!', ':', '"', '”', '’', '\'', ')', '…'])
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{AD}')
}

/// Words written with a hyphen inside a line somewhere in the chapter.
fn compounds(texts: &[&PageText]) -> HashSet<String> {
    let mut out = HashSet::new();
    for t in texts {
        for r in &t.lines {
            let line = &t.text[r.clone()];
            for w in line.split_whitespace() {
                let w =
                    w.trim_matches(|c: char| !c.is_alphanumeric() && !is_hyphen(c) && !is_mark(c));
                if w.chars().any(is_hyphen) && !w.ends_with(is_hyphen) && !w.starts_with(is_hyphen)
                {
                    out.insert(w.to_string());
                }
            }
        }
    }
    out
}

fn is_mark(c: char) -> bool {
    akkhor_flow::script::indic_cat(c).is_some()
}

struct Building {
    blocks: Vec<Block>,
    segments: Vec<Vec<Segment>>,
    /// Index of the last paragraph, while it may still continue.
    open: Option<usize>,
}

impl Building {
    /// Appends one line to block `b`, joining a line-end hyphen when the
    /// word is not a compound.
    fn push_line(
        &mut self,
        b: usize,
        page: usize,
        text: &PageText,
        line: usize,
        compounds: &HashSet<String>,
    ) {
        let Some(range) = text.lines.get(line).cloned() else {
            return;
        };
        let s = text.text[range.clone()].trim_end();
        let range = range.start..range.start + s.len();
        if s.is_empty() {
            return;
        }
        let block = &mut self.blocks[b];
        if !block.text.is_empty() {
            let last = block.text.chars().next_back().unwrap_or(' ');
            let first = s.chars().next().unwrap_or(' ');
            if is_hyphen(last) && (first.is_alphabetic() || is_mark(first)) {
                let head: String = block
                    .text
                    .rsplit(char::is_whitespace)
                    .next()
                    .unwrap_or("")
                    .to_string();
                let tail: String = s
                    .split(char::is_whitespace)
                    .next()
                    .unwrap_or("")
                    .to_string();
                let tail = tail.trim_end_matches(|c: char| !c.is_alphanumeric() && !is_mark(c));
                let joined = format!("{head}{tail}");
                // A doubled word (धीरे-धीरे, ঘর-ঘর) keeps its hyphen too.
                let doubled = head.trim_end_matches(is_hyphen) == tail;
                if !compounds.contains(&joined) && !doubled {
                    block.text.pop();
                }
            } else {
                block.text.push(' ');
            }
        }
        let at = block.text.len();
        block.text.push_str(s);
        self.segments[b].push(Segment { at, page, range });
    }

    fn new_block(&mut self, kind: FlowKind, style: BlockStyle) -> usize {
        self.blocks.push(Block {
            kind,
            text: String::new(),
            spans: Vec::new(),
            style,
            image: None,
        });
        self.segments.push(Vec::new());
        self.blocks.len() - 1
    }
}

/// Reflows a chapter: `pages[i]` is page `first + i`.
pub fn reflow(
    first: usize,
    texts: &[&PageText],
    pictures: &[Vec<Rect>],
    sizes: &[(f32, f32)],
) -> Reflow {
    // Pictures worth a block: not icons, not a background behind the text.
    let pictures: Vec<Vec<Rect>> = pictures
        .iter()
        .zip(sizes)
        .zip(texts)
        .map(|((pics, (w, h)), t)| {
            let area = f64::from(*w) * f64::from(*h);
            pics.iter()
                .filter(|r| {
                    let a = r.width() * r.height();
                    a > area * 0.015 && !(a > area * 0.85 && !t.text.trim().is_empty())
                })
                .copied()
                .collect()
        })
        .collect();
    let inputs: Vec<PageInput<'_>> = texts
        .iter()
        .zip(&pictures)
        .zip(sizes)
        .map(|((t, pics), (w, h))| PageInput {
            text: t,
            images: pics,
            page: Rect::new(0.0, 0.0, f64::from(*w), f64::from(*h)),
        })
        .collect();
    let layouts = structure::analyse(&inputs);
    let body = structure::body_size(&inputs);
    let compounds = compounds(texts);
    let mut out = Building {
        blocks: Vec::new(),
        segments: Vec::new(),
        open: None,
    };
    for (i, (layout, text)) in layouts.iter().zip(texts).enumerate() {
        let page = first + i;
        let mut notes: Vec<&structure::Block> = Vec::new();
        for b in &layout.blocks {
            match b.kind {
                BlockKind::Furniture => {}
                BlockKind::Footnote => notes.push(b),
                BlockKind::Image => {
                    let k = out.new_block(FlowKind::Image, BlockStyle::default());
                    out.blocks[k].image = Some(image_source(page, b.rect));
                    out.segments[k].push(Segment {
                        at: 0,
                        page,
                        range: 0..0,
                    });
                }
                BlockKind::Heading(level) => {
                    out.open = None;
                    let k = out.new_block(FlowKind::Heading(level), BlockStyle::default());
                    for &l in &b.lines {
                        out.push_line(k, page, text, l, &compounds);
                    }
                }
                BlockKind::Paragraph => {
                    // A paragraph cut by a column or page break goes on: the
                    // last one did not end a sentence, and this one does not
                    // start indented.
                    let k = match out.open {
                        Some(k) if !b.indented && !ends_sentence(&out.blocks[k].text) => k,
                        _ => out.new_block(FlowKind::Paragraph, BlockStyle::default()),
                    };
                    let start = out.blocks[k].text.len();
                    for &l in &b.lines {
                        out.push_line(k, page, text, l, &compounds);
                    }
                    style_spans(&mut out.blocks[k], start, b, text);
                    out.open = Some(k);
                }
            }
        }
        for n in notes {
            let style = BlockStyle {
                font_scale: Some(0.85),
                ..BlockStyle::default()
            };
            let k = out.new_block(FlowKind::Paragraph, style);
            for &l in &n.lines {
                out.push_line(k, page, text, l, &compounds);
            }
        }
    }
    let _ = body;
    // Paragraphs that kept nothing (only whitespace) are dropped.
    let mut document = Document::default();
    let mut segments = Vec::new();
    for (b, s) in out.blocks.into_iter().zip(out.segments) {
        if b.kind == FlowKind::Image || !b.text.trim().is_empty() {
            document.blocks.push(b);
            segments.push(s);
        }
    }
    Reflow {
        document,
        pages: first..first + texts.len(),
        segments,
    }
}

/// Bold and italic for the lines of `b` that are, in a body paragraph.
fn style_spans(block: &mut Block, start: usize, b: &structure::Block, text: &PageText) {
    let all_bold = b.lines.iter().all(|&l| text.line_info[l].bold);
    let all_italic = b.lines.iter().all(|&l| text.line_info[l].italic);
    if all_bold || all_italic {
        block.spans.push(StyleSpan {
            start,
            end: block.text.len(),
            style: Style {
                bold: all_bold,
                italic: all_italic,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::LineInfo;

    fn page(lines: &[(f64, f32, f64, &str)]) -> PageText {
        let mut t = PageText::default();
        for &(x0, base, x1, s) in lines {
            if !t.text.is_empty() {
                t.text.push('\n');
            }
            let start = t.text.len();
            t.text.push_str(s);
            t.lines.push(start..t.text.len());
            t.line_info.push(LineInfo {
                rect: Rect::new(x0, f64::from(base - 8.0), x1, f64::from(base + 2.5)),
                baseline: base,
                size: 10.0,
                ..LineInfo::default()
            });
        }
        t
    }

    #[test]
    fn hyphens_are_joined_but_compounds_kept() {
        let t = page(&[
            (40.0, 100.0, 380.0, "গ্রামের স্কুলে শি-"),
            (40.0, 112.0, 380.0, "ক্ষার্থীরা পাঠ-"),
            (40.0, 124.0, 380.0, "ইঞ্জিন দেখে। আবার পাঠ-ইঞ্জিন। ধীরে-"),
            (40.0, 136.0, 380.0, "ধীরে চলে।"),
        ]);
        let r = reflow(0, &[&t], &[vec![]], &[(420.0, 595.0)]);
        assert_eq!(r.document.blocks.len(), 1);
        assert_eq!(
            r.document.blocks[0].text,
            "গ্রামের স্কুলে শিক্ষার্থীরা পাঠ-ইঞ্জিন দেখে। আবার পাঠ-ইঞ্জিন। ধীরে-ধীরে চলে।"
        );
    }

    #[test]
    fn paragraph_continues_on_the_next_page() {
        let a = page(&[
            (40.0, 100.0, 380.0, "first page text that"),
            (40.0, 112.0, 380.0, "goes on"),
        ]);
        let b = page(&[
            (40.0, 100.0, 380.0, "and ends here."),
            (55.0, 112.0, 380.0, "New one."),
        ]);
        let r = reflow(3, &[&a, &b], &[vec![], vec![]], &[(420.0, 595.0); 2]);
        let texts: Vec<&str> = r.document.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            texts,
            ["first page text that goes on and ends here.", "New one."]
        );
        // Positions map both ways.
        let (page, off) = r
            .to_page(0, "first page text that goes on and ".len())
            .unwrap();
        assert_eq!(page, 4);
        assert_eq!(&b.text[off..off + 3], "end");
        assert_eq!(
            r.from_page(4, off),
            Some((0, "first page text that goes on and ".len()))
        );
        assert_eq!(r.block_page(1), Some(4));
    }

    #[test]
    fn image_sources_round_trip() {
        let s = image_source(7, Rect::new(10.0, 20.5, 300.0, 400.0));
        assert_eq!(
            parse_image_source(&s),
            Some((7, Rect::new(10.0, 20.5, 300.0, 400.0)))
        );
        assert_eq!(parse_image_source("chapter1/img.png"), None);
    }
}
