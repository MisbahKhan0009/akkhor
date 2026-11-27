//! Line breaking and pagination.
//!
//! Lines break at spaces and after dashes. A word too long for a line (a
//! long compound, a URL) is broken at an orthographic syllable boundary, so
//! a conjunct is never split across lines. Justification only widens
//! spaces; letters are never stretched, which matters for Bangla and
//! Devanagari where stretching would break the headline (matra/shirorekha).

use std::collections::VecDeque;
use std::ops::Range;

use crate::css::Align;
use crate::fonts::{FontId, FontSet, FontStyle, Synthetic};
use crate::hyphen::hyphenate;
use crate::image::ImageProvider;
use crate::model::{BlockKind, Document, Position, StyleSpan};
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
    /// Whether the line ends in a hyphen the layout put in (an English word
    /// carried on to the next line). That glyph alone sits at the line's end.
    pub fn hyphenated(&self) -> bool {
        self.glyphs
            .last()
            .is_some_and(|g| g.cluster == self.range.end)
    }

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

/// An image placed on a page, in page pixels.
#[derive(Clone, Debug)]
pub struct PlacedImage {
    pub block: usize,
    /// Source as written in the book; resolved by the [`ImageProvider`].
    pub src: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub lines: Vec<Line>,
    pub images: Vec<PlacedImage>,
}

impl Page {
    /// Reading position where this page starts: (block, byte offset).
    pub fn start(&self) -> Option<(usize, usize)> {
        let line = self.lines.first().map(|l| (l.block, l.range.start));
        let image = self.images.first().map(|i| (i.block, 0));
        match (line, image) {
            (Some(l), Some(i)) => Some(l.min(i)),
            (l, i) => l.or(i),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.images.is_empty()
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

/// Lays out a whole document into pages. Image blocks are skipped; use
/// [`paginate_with_images`] to place them.
pub fn paginate(doc: &Document, opts: &LayoutOptions, fonts: &FontSet) -> Vec<Page> {
    paginate_with_images(doc, opts, fonts, None)
}

/// Lays out a whole document into pages, placing images whose size
/// `images` can tell.
pub fn paginate_with_images(
    doc: &Document,
    opts: &LayoutOptions,
    fonts: &FontSet,
    images: Option<&dyn ImageProvider>,
) -> Vec<Page> {
    Paginator::new(doc, opts, fonts, images).collect()
}

/// Lays out a document one page at a time.
///
/// Pages come out as soon as they are full, so the first page of a long
/// chapter is ready after laying out only the blocks it needs. Stop
/// iterating (drop the paginator) to cancel, for example when the reader
/// changes the font size.
pub struct Paginator<'a> {
    doc: &'a Document,
    opts: &'a LayoutOptions,
    fonts: &'a FontSet,
    images: Option<&'a dyn ImageProvider>,
    next_block: usize,
    ready: VecDeque<Page>,
    current: Page,
    y: f32,
    prev_margin_bottom: f32,
    emitted: usize,
    finished: bool,
}

impl<'a> Paginator<'a> {
    pub fn new(
        doc: &'a Document,
        opts: &'a LayoutOptions,
        fonts: &'a FontSet,
        images: Option<&'a dyn ImageProvider>,
    ) -> Self {
        Self {
            doc,
            opts,
            fonts,
            images,
            next_block: 0,
            ready: VecDeque::new(),
            current: Page::default(),
            y: opts.margin_top,
            prev_margin_bottom: 0.0,
            emitted: 0,
            finished: false,
        }
    }

    /// How many blocks have been laid out so far.
    pub fn blocks_done(&self) -> usize {
        self.next_block
    }

    /// Lays out only as far as needed to find the page holding `pos` (a
    /// saved reading position) and returns its index and the page.
    pub fn page_for(self, pos: Position) -> Option<(usize, Page)> {
        let key = (pos.block, pos.offset);
        let mut prev = None;
        for (i, page) in self.enumerate() {
            if page.start().is_some_and(|s| s > key) {
                return prev;
            }
            prev = Some((i, page));
        }
        prev
    }

    fn new_page(&mut self) {
        let full = std::mem::take(&mut self.current);
        self.ready.push_back(full);
        self.y = self.opts.margin_top;
    }

    fn layout_block(&mut self, block_idx: usize) {
        let (doc, opts, fonts, images) = (self.doc, self.opts, self.fonts, self.images);
        let block = &doc.blocks[block_idx];
        let bottom = opts.height - opts.margin_bottom;
        let avail = (opts.width - 2.0 * opts.margin_x).max(1.0);
        if block.kind == BlockKind::Image {
            let Some((iw, ih)) = block
                .image
                .as_deref()
                .and_then(|src| images.and_then(|p| p.size(src)))
            else {
                return;
            };
            // Book pixels are CSS pixels: 16 of them to the body em.
            let scale = opts.font_size / 16.0;
            let max_h = (bottom - opts.margin_top).max(1.0);
            let fit = (avail / (iw as f32 * scale))
                .min(max_h / (ih as f32 * scale))
                .min(1.0);
            let (w, h) = (iw as f32 * scale * fit, ih as f32 * scale * fit);
            let page_empty = self.current.is_empty();
            if !page_empty {
                self.y += opts.paragraph_gap * opts.font_size;
            }
            if (self.y + h > bottom || block.style.page_break_before) && !page_empty {
                self.new_page();
                self.y = opts.margin_top;
            }
            self.current.images.push(PlacedImage {
                block: block_idx,
                src: block.image.clone().unwrap_or_default(),
                x: opts.margin_x + (avail - w) / 2.0,
                y: self.y,
                width: w,
                height: h,
            });
            self.y += h;
            // Keep a little air below a picture even when the CSS sets
            // paragraph margins to zero.
            self.prev_margin_bottom = opts.paragraph_gap;
            return;
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

        let page_empty = self.current.is_empty();
        if style.page_break_before && !page_empty {
            self.new_page();
            self.y = opts.margin_top;
        } else if !page_empty {
            // Vertical margins collapse: the larger of the two wins.
            let top = style.margin_top.unwrap_or(opts.paragraph_gap);
            self.y += top.max(self.prev_margin_bottom) * opts.font_size;
        }
        self.prev_margin_bottom = style.margin_bottom.unwrap_or(0.0);
        // A heading is not left alone at the foot of a page: it needs room
        // for itself and two lines of what follows.
        if matches!(block.kind, BlockKind::Heading(_))
            && !self.current.is_empty()
            && self.y + lh + 2.0 * opts.font_size * opts.line_height > bottom
        {
            self.new_page();
            self.y = opts.margin_top;
        }

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
                if self.y + lh > bottom && !self.current.is_empty() {
                    // A page does not end mid-word: hyphenated lines at its
                    // foot (two at most) go on with the rest of the word.
                    let mut carry = Vec::new();
                    while carry.len() < 2
                        && self.current.lines.last().is_some_and(Line::hyphenated)
                        && self.current.lines.len() + self.current.images.len() > 1
                    {
                        carry.extend(self.current.lines.pop());
                    }
                    self.new_page();
                    self.y = opts.margin_top;
                    for mut line in carry.into_iter().rev() {
                        let dy = self.y - line.top;
                        line.top += dy;
                        line.bottom += dy;
                        line.baseline += dy;
                        for g in &mut line.glyphs {
                            g.y += dy;
                        }
                        self.y = line.bottom;
                        self.current.lines.push(line);
                    }
                }
                let baseline = self.y + lh * 0.72;
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
                self.current.lines.push(Line {
                    block: block_idx,
                    range: seg_range.start + range.start..seg_range.start + range.end,
                    glyphs,
                    top: self.y,
                    bottom: self.y + lh,
                    baseline,
                    left,
                    right,
                });
                self.y += lh;
            }
        }
    }
}

impl Iterator for Paginator<'_> {
    type Item = Page;

    fn next(&mut self) -> Option<Page> {
        while self.ready.is_empty() && self.next_block < self.doc.blocks.len() {
            let b = self.next_block;
            self.next_block += 1;
            self.layout_block(b);
        }
        let page = self.ready.pop_front().or_else(|| {
            if self.finished {
                return None;
            }
            self.finished = true;
            // The last page; an empty document still gets one empty page.
            (!self.current.is_empty() || self.emitted == 0)
                .then(|| std::mem::take(&mut self.current))
        });
        self.emitted += page.is_some() as usize;
        page
    }
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
    // Running sum of advances, so any width is one subtraction.
    let mut prefix = Vec::with_capacity(glyphs.len() + 1);
    prefix.push(0.0f32);
    for g in glyphs {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + g.advance);
    }
    let width = |a: usize, b: usize| -> f32 {
        // Trailing spaces do not count towards the line width.
        let mut end = b;
        while end > a && bytes[end - 1] == b' ' {
            end -= 1;
        }
        prefix[gi(end)] - prefix[gi(a)]
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
    // A hyphen that is only nicer than a ragged line (`optional`) is not
    // used on a paragraph's last word, nor on its next-to-last line (the
    // rest of the paragraph would fit on one line).
    let hyphen_point = |start: usize, from: usize, optional: bool| -> Option<usize> {
        let avail = avail_for(start);
        let hy = hyphen?;
        let word_end = text[from..]
            .find(|c: char| !c.is_ascii_alphabetic())
            .map_or(text.len(), |e| from + e);
        if optional && !text[word_end..].trim_end().contains(char::is_whitespace) {
            return None;
        }
        let p = hyphenate(&text[from..word_end])
            .into_iter()
            .rev()
            .map(|p| from + p)
            .find(|&p| width(start, p) + hy.advance <= avail)?;
        if optional && width(p, text.len()) <= avail_for(p) {
            return None;
        }
        Some(p)
    };

    let mut lines = Vec::new();
    let mut start = 0;
    // Hyphenated lines in a row: at most two by choice (a word too long
    // for a line is still broken).
    let mut run = 0;
    while start < text.len() {
        let avail = avail_for(start);
        let left = shape.left + if start == 0 { shape.first_indent } else { 0.0 };
        // Furthest break that fits. Breaks are sorted, so start the scan at
        // the first one after `start`.
        let first = breaks.partition_point(|&b| b <= start);
        let mut end = None;
        for &b in &breaks[first..] {
            if width(start, b) <= avail {
                end = Some(b);
            } else {
                break;
            }
        }
        let mut hyphenated = false;
        let end = match end {
            // Lots of room left and an English word next: hyphenate it.
            Some(e) if e < text.len() && run < 2 && avail - width(start, e) > size * 2.0 => {
                match hyphen_point(start, e, true) {
                    Some(p) => {
                        hyphenated = true;
                        p
                    }
                    None => e,
                }
            }
            Some(e) => e,
            None => {
                if let Some(p) = hyphen_point(start, start, false) {
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
        run = if hyphenated { run + 1 } else { 0 };
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
    fn no_page_ends_on_a_hyphen() {
        let fonts = FontSet::bundled();
        let text = "Considerations notwithstanding, internationalization substantially \
                    complicates characterization, documentation, specification and \
                    implementation, extraordinarily. "
            .repeat(40);
        let doc = Document::from_plain_text(&text);
        let (mut ends_seen, mut carried) = (0, 0);
        // Many page heights, so page feet land on every kind of line.
        for height in (300..900).step_by(37) {
            let opts = LayoutOptions {
                width: 640.0,
                height: height as f32,
                ..Default::default()
            };
            let pages = paginate(&doc, &opts, &fonts);
            let mut next = 0;
            for (i, page) in pages.iter().enumerate() {
                for l in &page.lines {
                    // Whole text, in order, every line on its page.
                    assert_eq!(l.range.start, next, "height {height}, page {i}");
                    next = l.range.end;
                    assert!(l.bottom <= height as f32 - opts.margin_bottom + 0.01);
                    assert!(l.top >= opts.margin_top - 0.01);
                    for g in &l.glyphs {
                        assert!(g.y > l.top && g.y <= l.bottom, "glyph off its line");
                    }
                }
                for w in page.lines.windows(2) {
                    assert!(w[1].top >= w[0].bottom - 0.01, "lines overlap");
                }
                if i + 1 < pages.len() {
                    // A page of one or two lines keeps them: moving them
                    // would leave it empty.
                    let last = page.lines.last().unwrap();
                    if page.lines.len() >= 3 {
                        assert!(
                            !last.hyphenated(),
                            "height {height}: page {i} ends on a hyphen"
                        );
                        ends_seen += 1;
                    }
                }
                // One block of text: a line of room left at a page's foot
                // means lines were carried over.
                let room = height as f32 - opts.margin_bottom - page.lines.last().unwrap().bottom;
                if i + 1 < pages.len() && room >= opts.font_size * opts.line_height {
                    carried += 1;
                }
            }
            assert!(next >= doc.blocks[0].text.trim_end().len(), "text lost");
        }
        assert!(ends_seen > 50);
        // Some page feet did fall on a hyphenated line and were moved.
        assert!(carried > 0, "no page foot fell on a hyphen");
    }

    #[test]
    fn hyphens_follow_the_design_rules() {
        // Long words only, on a narrow page: plenty of chances to hyphenate.
        let fonts = FontSet::bundled();
        let text = "Considerations notwithstanding, internationalization substantially \
                    complicates characterization, documentation, specification and \
                    implementation, extraordinarily."
            .repeat(6);
        let doc = Document::from_plain_text(&text);
        // Wide enough for every word: a word too long for a line is still
        // broken, hyphen or not.
        for width in [640.0, 800.0, 1000.0] {
            let opts = LayoutOptions {
                width,
                ..Default::default()
            };
            let pages = paginate(&doc, &opts, &fonts);
            let t = &doc.blocks[0].text;
            let lines: Vec<_> = pages.iter().flat_map(|p| p.lines.iter()).collect();
            let hyphenated: Vec<bool> = lines
                .iter()
                .map(|l| {
                    l.range.end < t.len() && t.as_bytes()[l.range.end - 1].is_ascii_alphabetic()
                })
                .collect();
            assert!(
                hyphenated.iter().any(|&h| h),
                "width {width}: no hyphen at all"
            );
            // Never three in a row.
            assert!(
                !hyphenated.windows(3).any(|w| w.iter().all(|&h| h)),
                "width {width}: three hyphenated lines in a row"
            );
            // Never the paragraph's last word.
            let n = lines.len();
            assert!(
                !hyphenated[n - 2],
                "width {width}: last word hyphenated: {:?} / {:?}",
                &t[lines[n - 2].range.clone()],
                &t[lines[n - 1].range.clone()]
            );
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
    fn paginator_is_incremental_and_matches_paginate() {
        let fonts = FontSet::bundled();
        let para = "আমার সোনার বাংলা, আমি তোমায় ভালোবাসি। চিরদিন তোমার আকাশ, তোমার বাতাস। ";
        let text: Vec<String> = (0..2000)
            .map(|i| format!("{i}. {}", para.repeat(3)))
            .collect();
        let doc = Document::from_plain_text(&text.join("\n\n"));
        let opts = LayoutOptions::default();

        let mut p = Paginator::new(&doc, &opts, &fonts, None);
        let first = p.next().unwrap();
        assert!(
            p.blocks_done() < 10,
            "first page needed {} blocks",
            p.blocks_done()
        );

        let all = paginate(&doc, &opts, &fonts);
        assert!(all.len() > 200, "{} pages", all.len());
        assert_eq!(all[0].start(), first.start());
        assert_eq!(all[0].lines.len(), first.lines.len());

        // Jump to a saved position deep in the chapter.
        let target = Position::new(1234, 10);
        let (i, page) = Paginator::new(&doc, &opts, &fonts, None)
            .page_for(target)
            .unwrap();
        assert!(page.start().unwrap() <= (1234, 10));
        assert!(
            all.get(i + 1)
                .is_none_or(|n| n.start().unwrap() > (1234, 10))
        );
    }

    #[test]
    fn empty_document_has_one_page() {
        let fonts = FontSet::bundled();
        let pages = paginate(&Document::default(), &LayoutOptions::default(), &fonts);
        assert_eq!(pages.len(), 1);
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

    #[test]
    fn headings_are_not_left_at_the_foot_of_a_page() {
        let fonts = FontSet::bundled();
        let mut doc = Document::default();
        for i in 0..40 {
            let mut h = crate::model::Block::paragraph(format!("Heading {i}"));
            h.kind = BlockKind::Heading(2);
            doc.blocks.push(h);
            doc.blocks.push(crate::model::Block::paragraph(
                "Some words of a paragraph. ".repeat(6 + i % 5),
            ));
        }
        let opts = LayoutOptions {
            width: 600.0,
            height: 900.0,
            ..Default::default()
        };
        let pages = paginate(&doc, &opts, &fonts);
        assert!(pages.len() > 3);
        for p in &pages {
            let last = p.lines.last().unwrap();
            assert!(
                !matches!(doc.blocks[last.block].kind, BlockKind::Heading(_)),
                "a page ends with a heading"
            );
        }
    }
}
