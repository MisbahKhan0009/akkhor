//! Line breaking and pagination.
//!
//! Lines break at spaces and after dashes. A word too long for a line (a
//! long compound, a URL) is broken at an orthographic syllable boundary, so
//! a conjunct is never split across lines. Justification only widens
//! spaces; letters are never stretched, which matters for Bangla and
//! Devanagari where stretching would break the headline (matra/shirorekha).

use std::ops::Range;

use crate::fonts::{FontId, FontSet};
use crate::model::{BlockKind, Document};
use crate::script::syllable_starts;
use crate::shape::{ShapedGlyph, shape_paragraph};

#[derive(Clone, Debug)]
pub struct LayoutOptions {
    /// Page size in pixels.
    pub width: f32,
    pub height: f32,
    pub margin_x: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    /// Body text size in pixels.
    pub font_size: f32,
    /// Line height as a multiple of the font size. Indic scripts need more
    /// room than Latin for vowel signs above and below; 1.7 suits both.
    pub line_height: f32,
    /// Space between paragraphs, as a multiple of the font size.
    pub paragraph_gap: f32,
    pub justify: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            width: 1080.0,
            height: 1920.0,
            margin_x: 72.0,
            margin_top: 96.0,
            margin_bottom: 96.0,
            font_size: 46.0,
            line_height: 1.7,
            paragraph_gap: 0.6,
            justify: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlacedGlyph {
    pub font: FontId,
    pub glyph: u32,
    /// Pen position in page pixels; `y` is the baseline.
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

#[derive(Clone, Debug)]
pub struct Line {
    /// Index of the block this line belongs to.
    pub block: usize,
    /// Byte range of the block's text shown on this line.
    pub range: Range<usize>,
    pub glyphs: Vec<PlacedGlyph>,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub lines: Vec<Line>,
}

impl Page {
    /// Reading position where this page starts: (block, byte offset).
    pub fn start(&self) -> Option<(usize, usize)> {
        self.lines.first().map(|l| (l.block, l.range.start))
    }
}

fn heading_scale(kind: BlockKind) -> f32 {
    match kind {
        BlockKind::Heading(1) => 1.6,
        BlockKind::Heading(2) => 1.4,
        BlockKind::Heading(3) => 1.25,
        BlockKind::Heading(_) => 1.1,
        _ => 1.0,
    }
}

/// Lays out a whole document into pages.
pub fn paginate(doc: &Document, opts: &LayoutOptions, fonts: &FontSet) -> Vec<Page> {
    let mut pages = vec![Page::default()];
    let bottom = opts.height - opts.margin_bottom;
    let avail = (opts.width - 2.0 * opts.margin_x).max(1.0);
    let mut y = opts.margin_top;

    for (block_idx, block) in doc.blocks.iter().enumerate() {
        let size = opts.font_size * heading_scale(block.kind);
        let lh = size * opts.line_height;
        let indent = match block.kind {
            BlockKind::Quote | BlockKind::ListItem => size * 1.5,
            _ => 0.0,
        };
        let justify = opts.justify && matches!(block.kind, BlockKind::Paragraph | BlockKind::Quote);

        if !pages.last().is_some_and(|p| p.lines.is_empty()) {
            y += opts.font_size * opts.paragraph_gap;
        }

        // Forced line breaks split the block into segments laid out separately.
        let mut seg_start = 0;
        for segment in block.text.split('\n') {
            let seg_range = seg_start..seg_start + segment.len();
            seg_start = seg_range.end + 1;
            let glyphs = shape_paragraph(segment, size, fonts);
            for (range, placed) in break_lines(
                segment,
                &glyphs,
                avail - indent,
                justify,
                opts.margin_x + indent,
            ) {
                if y + lh > bottom && !pages.last().is_some_and(|p| p.lines.is_empty()) {
                    pages.push(Page::default());
                    y = opts.margin_top;
                }
                let baseline = y + lh * 0.72;
                pages
                    .last_mut()
                    .expect("at least one page")
                    .lines
                    .push(Line {
                        block: block_idx,
                        range: seg_range.start + range.start..seg_range.start + range.end,
                        glyphs: placed
                            .into_iter()
                            .map(|mut g| {
                                g.y += baseline;
                                g
                            })
                            .collect(),
                    });
                y += lh;
            }
        }
    }
    if pages.len() > 1 && pages.last().is_some_and(|p| p.lines.is_empty()) {
        pages.pop();
    }
    pages
}

/// Breaks one shaped segment into lines. Returns each line's byte range and
/// its glyphs with `x` set and `y` relative to the baseline.
fn break_lines(
    text: &str,
    glyphs: &[ShapedGlyph],
    avail: f32,
    justify: bool,
    left: f32,
) -> Vec<(Range<usize>, Vec<PlacedGlyph>)> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    // Break opportunities: after a run of spaces, or after a dash when both
    // sides are real words (so বই-ই or e-mail stay together).
    let mut breaks: Vec<usize> = Vec::new();
    for (i, c) in text.char_indices().skip(1) {
        let prev = text[..i].chars().next_back().unwrap_or(' ');
        let after_space = prev == ' ' && c != ' ';
        let after_dash = matches!(prev, '-' | '\u{2013}' | '\u{2014}')
            && c != ' '
            && syllables_around(text, i) >= (2, 2);
        if after_space || after_dash {
            breaks.push(i);
        }
    }
    breaks.push(text.len());

    // Glyph index of the first glyph whose cluster is >= byte offset.
    let gi = |offset: usize| glyphs.partition_point(|g| g.cluster < offset);
    let width = |a: usize, b: usize| -> f32 {
        // Trailing spaces do not count towards the line width.
        let mut end = b;
        while end > a && bytes[end - 1] == b' ' {
            end -= 1;
        }
        glyphs[gi(a)..gi(end)].iter().map(|g| g.advance).sum()
    };

    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        // Furthest break that fits.
        let mut end = None;
        for &b in breaks.iter().filter(|&&b| b > start) {
            if width(start, b) <= avail {
                end = Some(b);
            } else {
                break;
            }
        }
        let end = match end {
            Some(e) => e,
            None => {
                // The next word alone is too long: break it between syllables.
                let word_end = breaks
                    .iter()
                    .copied()
                    .find(|&b| b > start)
                    .unwrap_or(text.len());
                let syl: Vec<usize> = syllable_starts(&text[start..word_end])
                    .into_iter()
                    .map(|s| s + start)
                    .filter(|&s| s > start)
                    .collect();
                let mut cut = syl.first().copied().unwrap_or(word_end);
                for s in syl {
                    if width(start, s) <= avail {
                        cut = s;
                    } else {
                        break;
                    }
                }
                cut
            }
        };
        let is_last = end >= text.len();
        lines.push((
            start..end,
            place(
                text,
                glyphs,
                gi(start),
                gi(end),
                avail,
                justify && !is_last,
                left,
            ),
        ));
        start = end;
    }
    lines
}

/// Syllables in the word fragment before the dash at `i - 1` and after `i`.
fn syllables_around(text: &str, i: usize) -> (usize, usize) {
    let before_start = text[..i].rfind(' ').map_or(0, |p| p + 1);
    let after_end = text[i..].find(' ').map_or(text.len(), |p| i + p);
    let count = |s: &str| {
        crate::script::syllables(s)
            .into_iter()
            .filter(|r| s[r.clone()].chars().any(char::is_alphanumeric))
            .count()
    };
    (count(&text[before_start..i]), count(&text[i..after_end]))
}

fn place(
    text: &str,
    glyphs: &[ShapedGlyph],
    from: usize,
    to: usize,
    avail: f32,
    justify: bool,
    left: f32,
) -> Vec<PlacedGlyph> {
    let line = &glyphs[from..to];
    let is_space = |g: &ShapedGlyph| text.as_bytes().get(g.cluster) == Some(&b' ');
    // Ignore trailing spaces for width and justification.
    let mut visible = line.len();
    while visible > 0 && is_space(&line[visible - 1]) {
        visible -= 1;
    }
    let line = &line[..visible];
    let natural: f32 = line.iter().map(|g| g.advance).sum();
    let spaces = line.iter().filter(|g| is_space(g)).count();
    let mut extra = 0.0;
    if justify && spaces > 0 {
        let per = (avail - natural) / spaces as f32;
        // Very loose lines look worse justified than ragged.
        let size = line.first().map_or(16.0, |g| g.size);
        if per > 0.0 && per < size * 0.45 {
            extra = per;
        }
    }
    let mut pen = left;
    line.iter()
        .map(|g| {
            let placed = PlacedGlyph {
                font: g.font,
                glyph: g.glyph,
                x: pen + g.x_offset,
                y: -g.y_offset,
                size: g.size,
            };
            pen += g.advance + if is_space(g) { extra } else { 0.0 };
            placed
        })
        .collect()
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;
    use crate::model::Document;

    #[test]
    fn long_word_breaks_between_syllables() {
        let fonts = FontSet::bundled();
        // One long Bangla "word" in a very narrow column.
        let word = "ক্ষমাপ্রার্থনাস্বরূপসংক্রান্ত";
        let doc = Document::from_plain_text(word);
        let opts = LayoutOptions {
            width: 260.0,
            margin_x: 10.0,
            font_size: 30.0,
            ..Default::default()
        };
        let pages = paginate(&doc, &opts, &fonts);
        let lines = &pages[0].lines;
        assert!(lines.len() > 1, "should wrap");
        let starts = syllable_starts(word);
        for l in lines {
            assert!(
                starts.contains(&l.range.start),
                "line starts mid-syllable: {:?}",
                &word[l.range.clone()]
            );
        }
    }

    #[test]
    fn paginates_and_keeps_positions() {
        let fonts = FontSet::bundled();
        let para = "আমার সোনার বাংলা, আমি তোমায় ভালোবাসি। ".repeat(40);
        let text = format!("{para}\n\n{}", "हिन्दी एक भाषा है। ".repeat(40));
        let doc = Document::from_plain_text(&text);
        let pages = paginate(&doc, &LayoutOptions::default(), &fonts);
        assert!(!pages.is_empty());
        // Every line range is a valid char boundary in its block.
        for p in &pages {
            for l in &p.lines {
                let t = &doc.blocks[l.block].text;
                assert!(t.is_char_boundary(l.range.start) && t.is_char_boundary(l.range.end));
                assert!(l.glyphs.iter().all(|g| g.x >= 0.0 && g.x < 1080.0));
            }
        }
    }
}
