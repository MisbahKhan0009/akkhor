//! Line breaking and pagination.
//!
//! Lines break at spaces and after dashes. A word too long for a line (a
//! long compound, a URL) is broken at an orthographic syllable boundary, so
//! a conjunct is never split across lines. Justification only widens
//! spaces; letters are never stretched, which matters for Bangla and
//! Devanagari where stretching would break the headline (matra/shirorekha).

use std::ops::Range;

use crate::css::Align;
use crate::fonts::{FontId, FontSet, FontStyle, Synthetic};
use crate::hyphen::hyphenate;
use crate::model::StyleSpan;
use crate::model::{BlockKind, Document};
use crate::script::syllable_starts;
use crate::shape::{ShapedGlyph, shape_paragraph, shape_styled};

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
    /// Where the glyph is drawn, in page pixels; `y` is on the baseline
    /// (plus any mark offset).
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// Byte offset in the block text of the cluster this glyph belongs to.
    pub cluster: usize,
    /// Pen position before this glyph and its advance (including any
    /// justification space), for hit-testing and selection.
    pub pen: f32,
    pub advance: f32,
    /// Emboldening/slant to apply because no real bold/italic face exists.
    pub synthetic: Synthetic,
}

#[derive(Clone, Debug)]
pub struct Line {
    /// Index of the block this line belongs to.
    pub block: usize,
    /// Byte range of the block's text shown on this line.
    pub range: Range<usize>,
    pub glyphs: Vec<PlacedGlyph>,
    /// Vertical extent of the line box and its baseline, in page pixels.
    pub top: f32,
    pub bottom: f32,
    pub baseline: f32,
    /// Horizontal extent of the visible text.
    pub left: f32,
    pub right: f32,
}

impl Line {
    /// The x position where the text at byte `offset` starts on this line.
    /// Offsets at or past the end of the line map to its right edge.
    pub fn x_of(&self, offset: usize) -> f32 {
        if offset <= self.range.start {
            return self.left;
        }
        self.glyphs
            .iter()
            .find(|g| g.cluster >= offset)
            .map_or(self.right, |g| g.pen)
    }
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

    let mut prev_margin_bottom = 0.0f32;
    for (block_idx, block) in doc.blocks.iter().enumerate() {
        if block.kind == BlockKind::Image {
            continue; // images are laid out from Phase 1.6
        }
        let style = &block.style;
        let size = opts.font_size
            * style
                .font_scale
                .unwrap_or_else(|| heading_scale(block.kind));
        let lh = size * opts.line_height;
        let indent = match block.kind {
            BlockKind::Quote | BlockKind::ListItem => size * 1.5,
            _ => 0.0,
        };
        let align = match style.align {
            Some(a) => a,
            None if opts.justify
                && matches!(block.kind, BlockKind::Paragraph | BlockKind::Quote) =>
            {
                Align::Justify
            }
            None => Align::Start,
        };
        let first_indent = style.indent.unwrap_or(0.0) * opts.font_size;

        let page_empty = pages.last().is_some_and(|p| p.lines.is_empty());
        if style.page_break_before && !page_empty {
            pages.push(Page::default());
            y = opts.margin_top;
        } else if !page_empty {
            // Vertical margins collapse: the larger of the two wins.
            let top = style.margin_top.unwrap_or(opts.paragraph_gap);
            y += top.max(prev_margin_bottom) * opts.font_size;
        }
        prev_margin_bottom = style.margin_bottom.unwrap_or(0.0);

        // Forced line breaks split the block into segments laid out separately.
        let mut seg_start = 0;
        for (seg_idx, segment) in block.text.split('\n').enumerate() {
            let seg_range = seg_start..seg_start + segment.len();
            seg_start = seg_range.end + 1;
            // Style spans of this segment, made relative to it.
            let spans: Vec<StyleSpan> = block
                .spans
                .iter()
                .filter(|sp| sp.end > seg_range.start && sp.start < seg_range.end)
                .map(|sp| StyleSpan {
                    start: sp.start.max(seg_range.start) - seg_range.start,
                    end: sp.end.min(seg_range.end) - seg_range.start,
                    style: sp.style,
                })
                .collect();
            let glyphs = shape_styled(segment, &spans, FontStyle::REGULAR, size, fonts);
            // Only English words are hyphenated; don't load a Latin font for
            // a segment without any.
            let hyphen = if segment.bytes().any(|b| b.is_ascii_alphabetic()) {
                shape_paragraph("-", size, fonts).first().copied()
            } else {
                None
            };
            let shape = LineShape {
                avail: avail - indent,
                left: opts.margin_x + indent,
                first_indent: if seg_idx == 0 { first_indent } else { 0.0 },
                align,
            };
            for (range, placed) in break_lines(segment, &glyphs, &shape, hyphen) {
                if y + lh > bottom && !pages.last().is_some_and(|p| p.lines.is_empty()) {
                    pages.push(Page::default());
                    y = opts.margin_top;
                }
                let baseline = y + lh * 0.72;
                let glyphs: Vec<PlacedGlyph> = placed
                    .into_iter()
                    .map(|mut g| {
                        g.y += baseline;
                        g.cluster += seg_range.start;
                        g
                    })
                    .collect();
                let left = glyphs.first().map_or(opts.margin_x + indent, |g| g.pen);
                let right = glyphs.last().map_or(left, |g| g.pen + g.advance);
                pages
                    .last_mut()
                    .expect("at least one page")
                    .lines
                    .push(Line {
                        block: block_idx,
                        range: seg_range.start + range.start..seg_range.start + range.end,
                        glyphs,
                        top: y,
                        bottom: y + lh,
                        baseline,
                        left,
                        right,
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

/// Characters a line must not start with.
fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '।' | '॥'
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
            | '»'
            | '”'
            | '’'
            | '—'
            | '–'
    )
}

/// Characters a line must not end with.
fn no_line_end(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '«' | '“' | '‘')
}

/// Breaks one shaped segment into lines. Returns each line's byte range and
/// its glyphs with `x` set and `y` relative to the baseline. `hyphen` is the
/// shaped hyphen glyph used when an English word is hyphenated.
/// The horizontal shape of the lines of one segment.
struct LineShape {
    /// Width available to text, and its left edge.
    avail: f32,
    left: f32,
    /// Extra indent of the first line (negative for a hanging indent).
    first_indent: f32,
    align: Align,
}

fn break_lines(
    text: &str,
    glyphs: &[ShapedGlyph],
    shape: &LineShape,
    hyphen: Option<ShapedGlyph>,
) -> Vec<(Range<usize>, Vec<PlacedGlyph>)> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    // Break opportunities: after a run of spaces, or after a dash when both
    // sides are real words (so বই-ই or e-mail stay together). Never before
    // closing punctuation (a line never starts with ।) or after opening
    // punctuation.
    let mut breaks: Vec<usize> = Vec::new();
    for (i, c) in text.char_indices().skip(1) {
        let prev = text[..i].chars().next_back().unwrap_or(' ');
        let after_space = prev == ' ' && c != ' ';
        let after_dash = matches!(prev, '-' | '\u{2013}' | '\u{2014}')
            && c != ' '
            && syllables_around(text, i) >= (2, 2);
        if !(after_space || after_dash) || no_line_start(c) {
            continue;
        }
        let last_word_char = text[..i].trim_end().chars().next_back();
        if last_word_char.is_some_and(no_line_end) {
            continue;
        }
        breaks.push(i);
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
    let size = glyphs.first().map_or(16.0, |g| g.size);
    // The first line is shorter (or longer) by the first-line indent.
    let avail_for = |start: usize| {
        if start == 0 {
            (shape.avail - shape.first_indent).max(size)
        } else {
            shape.avail
        }
    };

    // Where the English word starting at `from` may be hyphenated so that the
    // line from `start` still fits (with the hyphen). Picks the latest point.
    let hyphen_point = |start: usize, from: usize| -> Option<usize> {
        let avail = avail_for(start);
        let hy = hyphen?;
        let word_end = text[from..]
            .find(|c: char| !c.is_ascii_alphabetic())
            .map_or(text.len(), |e| from + e);
        hyphenate(&text[from..word_end])
            .into_iter()
            .rev()
            .map(|p| from + p)
            .find(|&p| width(start, p) + hy.advance <= avail)
    };

    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let avail = avail_for(start);
        let left = shape.left + if start == 0 { shape.first_indent } else { 0.0 };
        // Furthest break that fits.
        let mut end = None;
        for &b in breaks.iter().filter(|&&b| b > start) {
            if width(start, b) <= avail {
                end = Some(b);
            } else {
                break;
            }
        }
        let mut hyphenated = false;
        let end = match end {
            // Lots of room left and an English word next: hyphenate it.
            Some(e) if e < text.len() && avail - width(start, e) > size * 2.0 => {
                match hyphen_point(start, e) {
                    Some(p) => {
                        hyphenated = true;
                        p
                    }
                    None => e,
                }
            }
            Some(e) => e,
            None => {
                if let Some(p) = hyphen_point(start, start) {
                    hyphenated = true;
                    p
                } else {
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
            }
        };
        let is_last = end >= text.len();
        // The last line of a justified paragraph is set ragged.
        let align = match shape.align {
            Align::Justify if is_last => Align::Start,
            a => a,
        };
        let mut placed = place(
            text,
            glyphs,
            gi(start),
            gi(end),
            avail
                - if hyphenated {
                    hyphen.map_or(0.0, |h| h.advance)
                } else {
                    0.0
                },
            align,
            left,
        );
        if hyphenated && let Some(h) = hyphen {
            let pen = placed.last().map_or(left, |g| g.pen + g.advance);
            placed.push(PlacedGlyph {
                font: h.font,
                glyph: h.glyph,
                x: pen + h.x_offset,
                y: -h.y_offset,
                size: h.size,
                cluster: end,
                pen,
                advance: h.advance,
                synthetic: placed.last().map_or(h.synthetic, |g| g.synthetic),
            });
        }
        lines.push((start..end, placed));
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
    align: Align,
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
    if align == Align::Justify && spaces > 0 {
        let per = (avail - natural) / spaces as f32;
        // Very loose lines look worse justified than ragged.
        let size = line.first().map_or(16.0, |g| g.size);
        if per > 0.0 && per < size * 0.6 {
            extra = per;
        }
    }
    let free = (avail - natural).max(0.0);
    let mut pen = left
        + match align {
            Align::Center => free / 2.0,
            Align::End => free,
            _ => 0.0,
        };
    line.iter()
        .map(|g| {
            let advance = g.advance + if is_space(g) { extra } else { 0.0 };
            let placed = PlacedGlyph {
                font: g.font,
                glyph: g.glyph,
                x: pen + g.x_offset,
                y: -g.y_offset,
                size: g.size,
                cluster: g.cluster,
                pen,
                advance,
                synthetic: g.synthetic,
            };
            pen += advance;
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
    fn lines_never_start_with_closing_punctuation() {
        let fonts = FontSet::bundled();
        let text = "আমি বাংলায় গান গাই । ".repeat(30);
        let doc = Document::from_plain_text(&text);
        let pages = paginate(&doc, &LayoutOptions::default(), &fonts);
        for l in &pages[0].lines {
            let t = &doc.blocks[0].text[l.range.clone()];
            assert!(!t.starts_with('।'), "line starts with danda: {t}");
        }
    }

    #[test]
    fn english_words_get_hyphenated() {
        let fonts = FontSet::bundled();
        let text = "Internationalization considerations notwithstanding, hyphenation substantially improves justification. ".repeat(8);
        let doc = Document::from_plain_text(&text);
        let opts = LayoutOptions {
            width: 700.0,
            ..Default::default()
        };
        let pages = paginate(&doc, &opts, &fonts);
        let lines = &pages[0].lines;
        let hyphenated = lines
            .iter()
            .filter(|l| {
                let t = &doc.blocks[0].text;
                l.range.end < t.len() && t.as_bytes()[l.range.end - 1].is_ascii_alphabetic()
            })
            .count();
        assert!(hyphenated > 0, "expected at least one hyphenated line");
        // Text is preserved: lines cover the block without gaps.
        for w in lines.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start);
        }
    }

    #[test]
    fn css_alignment_indent_and_page_breaks() {
        let fonts = FontSet::bundled();
        let doc = Document::from_html(
            r#"<style>.c{text-align:center; text-indent:0} .r{text-align:right; text-indent:0} p{text-indent:2em}
               .brk{page-break-before:always}</style>
               <p class="c">মাঝখানে</p><p class="r">ডানে</p>
               <p>প্রথম লাইনে ইন্ডেন্ট থাকবে।</p><p class="brk">নতুন পাতা</p>"#,
        );
        let opts = LayoutOptions::default();
        let pages = paginate(&doc, &opts, &fonts);
        assert_eq!(pages.len(), 2, "page break starts a new page");
        let l = &pages[0].lines;
        let mid = opts.width / 2.0;
        let center = (l[0].left + l[0].right) / 2.0;
        assert!((center - mid).abs() < 2.0, "centred: {center} vs {mid}");
        assert!(
            (l[1].right - (opts.width - opts.margin_x)).abs() < 2.0,
            "right-aligned"
        );
        assert!(
            (l[2].left - (opts.margin_x + 2.0 * opts.font_size)).abs() < 1.0,
            "first-line indent"
        );
        assert_eq!(pages[1].lines[0].block, 3);
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
