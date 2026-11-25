//! Page structure: the lines of a page's text grouped into blocks
//! (headings, paragraphs, footnotes, images, furniture) in reading order.
//!
//! Pure functions over [`PageText`] line boxes and image boxes, so every
//! rule can be tested on hand-made lines. The steps:
//!
//! 1. **Reading order** by recursive XY-cut: a vertical gap that runs the
//!    full height of a region splits it into columns (left to right);
//!    otherwise the region is read top to bottom, band by band. Columns are
//!    tried first, so two columns whose paragraph gaps happen to line up are
//!    not read across.
//! 2. **Paragraphs** inside each region: a new block starts at a vertical
//!    gap wider than the region's usual line gap, at a first-line indent,
//!    after a short last line of justified text, or where size or weight
//!    changes.
//! 3. **Kinds**, over a whole chapter so that repetition and the body size
//!    can be seen: furniture (tagged artifacts, or a line in the top or
//!    bottom band that repeats on nearby pages or is only a page number),
//!    footnotes (small text below the body), headings (larger or bold and
//!    short; levels by size).

use std::ops::Range;

use hayro::kurbo::Rect;

use crate::text::{LineInfo, PageText};

/// What a block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Heading level 1 to 6.
    Heading(u8),
    Paragraph,
    Footnote,
    Image,
    /// Running header or footer, page number: left out of a reflow.
    Furniture,
}

/// A block of one page.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    /// Indices into the page's [`PageText::lines`], top to bottom. Empty
    /// for an image.
    pub lines: Vec<usize>,
    /// Box on the page, in points (top-left origin).
    pub rect: Rect,
    /// Region (column or band) of the page, numbered in reading order.
    pub region: usize,
    /// The block's first line starts indented from its region's left edge.
    pub indented: bool,
}

/// The blocks of one page, in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLayout {
    pub blocks: Vec<Block>,
    /// Page box, in points.
    pub page: Rect,
}

/// One page's input: its text, its picture boxes and its size.
pub struct PageInput<'a> {
    pub text: &'a PageText,
    pub images: &'a [Rect],
    pub page: Rect,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Line(usize),
    Image(usize),
}

/// Analyses a run of pages (a chapter).
pub fn analyse(pages: &[PageInput<'_>]) -> Vec<PageLayout> {
    let body = body_size(pages);
    let mut out: Vec<PageLayout> = pages.iter().map(|p| layout_page(p, body)).collect();
    mark_furniture(pages, &mut out, body);
    mark_footnotes(pages, &mut out, body);
    mark_headings(pages, &mut out, body);
    out
}

/// The body text size: the size that most characters are set in.
pub fn body_size(pages: &[PageInput<'_>]) -> f32 {
    let mut by_size: Vec<(f32, usize)> = Vec::new();
    for p in pages {
        for (info, range) in p.text.line_info.iter().zip(&p.text.lines) {
            let n = p.text.text[range.clone()]
                .chars()
                .filter(|c| !c.is_whitespace())
                .count();
            let s = (info.size * 2.0).round() / 2.0;
            match by_size.iter_mut().find(|(x, _)| *x == s) {
                Some((_, c)) => *c += n,
                None => by_size.push((s, n)),
            }
        }
    }
    by_size
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map_or(10.0, |(s, _)| s.max(1.0))
}

fn line_text(text: &PageText, line: usize) -> &str {
    text.lines.get(line).map_or("", |r| &text.text[r.clone()])
}

fn info(text: &PageText, line: usize) -> &LineInfo {
    &text.line_info[line]
}

// --- 1. reading order -------------------------------------------------------

fn rect_of(p: &PageInput<'_>, item: Item) -> Rect {
    match item {
        Item::Line(i) => info(p.text, i).rect,
        Item::Image(i) => p.images[i],
    }
}

/// Items in reading order, each with its region number.
fn reading_order(p: &PageInput<'_>, body: f32) -> Vec<(usize, Item)> {
    let mut items: Vec<Item> = (0..p.text.lines.len())
        .filter(|&i| !line_text(p.text, i).trim().is_empty())
        .map(Item::Line)
        .collect();
    items.extend((0..p.images.len()).map(Item::Image));
    let mut out = Vec::with_capacity(items.len());
    let mut region = 0;
    cut(p, items, body, 0, &mut region, &mut out);
    out
}

/// Columns nested deeper than this are read as one region: real pages have
/// a few, and a hostile page with thousands would otherwise overflow the
/// stack (an abort that no panic guard catches).
const MAX_CUT_DEPTH: usize = 24;

fn cut(
    p: &PageInput<'_>,
    mut items: Vec<Item>,
    body: f32,
    depth: usize,
    region: &mut usize,
    out: &mut Vec<(usize, Item)>,
) {
    if items.is_empty() {
        return;
    }
    let depth = depth + 1;
    // Columns: the widest vertical gap crossing every item of the region.
    if items.len() > 1
        && depth < MAX_CUT_DEPTH
        && let Some(x) = vertical_gap(p, &items, f64::from(body) * 0.6)
    {
        let (left, right): (Vec<Item>, Vec<Item>) = items
            .into_iter()
            .partition(|&it| rect_of(p, it).center().x < x);
        cut(p, left, body, depth, region, out);
        cut(p, right, body, depth, region, out);
        return;
    }
    // Bands, top to bottom: items overlapping vertically share a band. A
    // band that splits into columns is a region of its own; runs of plain
    // bands make one region.
    items.sort_by(|&a, &b| {
        let (ra, rb) = (rect_of(p, a), rect_of(p, b));
        ra.y0.total_cmp(&rb.y0).then(ra.x0.total_cmp(&rb.x0))
    });
    let mut bands: Vec<(f64, Vec<Item>)> = Vec::new();
    for it in items {
        let r = rect_of(p, it);
        match bands.last_mut() {
            // Lines overlap their neighbours a little (the boxes run from
            // ascender to descender): a band continues while the overlap is
            // over a third of the smaller height.
            Some((bottom, band))
                if r.y0 < *bottom - r.height().min(*bottom - band_top(p, band)) / 3.0 =>
            {
                *bottom = bottom.max(r.y1);
                band.push(it);
            }
            _ => bands.push((r.y1, vec![it])),
        }
    }
    let mut plain: Vec<Item> = Vec::new();
    let flush = |plain: &mut Vec<Item>, region: &mut usize, out: &mut Vec<(usize, Item)>| {
        if !plain.is_empty() {
            for it in plain.drain(..) {
                out.push((*region, it));
            }
            *region += 1;
        }
    };
    // Consecutive bands that each split into columns are read as one set of
    // columns (a1 a2 then b1 b2), not band by band (a1 b1 a2 b2).
    let mut columns: Vec<Vec<Item>> = Vec::new();
    let flush_columns =
        |columns: &mut Vec<Vec<Item>>, region: &mut usize, out: &mut Vec<(usize, Item)>| {
            if columns.is_empty() {
                return;
            }
            let all: Vec<Item> = columns.iter().flatten().copied().collect();
            if columns.len() > 1 && vertical_gap(p, &all, f64::from(body) * 0.6).is_some() {
                cut(p, all, body, depth, region, out);
            } else {
                for band in columns.iter() {
                    cut(p, band.clone(), body, depth, region, out);
                }
            }
            columns.clear();
        };
    // The gutter of the current run of column bands. A band with text on
    // one side only (a column ending early, a heading's white space) stays
    // in the run as long as nothing crosses the gutter.
    // The run's gutter narrows to what every band leaves open.
    let min = f64::from(body) * 0.6;
    let mut gutter: Option<(f64, f64)> = None;
    let open_part = |band: &[Item], (lo, hi): (f64, f64)| {
        let (mut lo, mut hi) = (lo, hi);
        // Each item belongs to the side its centre is on and pushes that
        // side's edge of the gutter.
        let mid = (lo + hi) / 2.0;
        for &it in band {
            let r = rect_of(p, it);
            if r.center().x < mid {
                lo = lo.max(r.x1);
            } else {
                hi = hi.min(r.x0);
            }
        }
        (hi - lo >= min).then_some((lo, hi))
    };
    let owns: Vec<Option<(f64, f64)>> = bands
        .iter()
        .map(|(_, b)| {
            if b.len() > 1 && depth < MAX_CUT_DEPTH {
                gap_interval(p, b, min)
            } else {
                None
            }
        })
        .collect();
    for (bi, (_, mut band)) in bands.into_iter().enumerate() {
        let own = owns[bi];
        // A band with text on one side only, just before a run of columns
        // (the first line of the right column, set lower than the left
        // column's heading), joins that run.
        if gutter.is_none() && own.is_none() {
            gutter = owns[bi + 1..]
                .iter()
                .flatten()
                .next()
                .copied()
                .filter(|g| open_part(&band, *g).is_some());
            if gutter.is_some() {
                flush(&mut plain, region, out);
            }
        }
        let kept = gutter.and_then(|g| open_part(&band, g));
        match (kept, own) {
            (Some(g), _) => {
                gutter = Some(g);
                columns.push(band);
            }
            (None, Some(x)) => {
                flush(&mut plain, region, out);
                flush_columns(&mut columns, region, out);
                gutter = Some(x);
                columns.push(band);
            }
            _ => {
                flush_columns(&mut columns, region, out);
                gutter = None;
                band.sort_by(|&a, &b| rect_of(p, a).x0.total_cmp(&rect_of(p, b).x0));
                plain.extend(band);
            }
        }
    }
    flush_columns(&mut columns, region, out);
    flush(&mut plain, region, out);
}

fn band_top(p: &PageInput<'_>, band: &[Item]) -> f64 {
    band.iter()
        .map(|&it| rect_of(p, it).y0)
        .fold(f64::INFINITY, f64::min)
}

/// The middle of the widest gap, at least `min` wide, that no item crosses.
fn vertical_gap(p: &PageInput<'_>, items: &[Item], min: f64) -> Option<f64> {
    gap_interval(p, items, min).map(|(lo, hi)| (lo + hi) / 2.0)
}

/// The widest gap, at least `min` wide, that no item crosses: (left, right).
fn gap_interval(p: &PageInput<'_>, items: &[Item], min: f64) -> Option<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = items
        .iter()
        .map(|&it| {
            let r = rect_of(p, it);
            (r.x0, r.x1)
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut best: Option<(f64, f64)> = None;
    let mut reach = spans[0].1;
    for &(x0, x1) in &spans[1..] {
        if x0 - reach >= min && best.is_none_or(|(lo, hi)| x0 - reach > hi - lo) {
            best = Some((reach, x0));
        }
        reach = reach.max(x1);
    }
    best
}

// --- 2. paragraphs -----------------------------------------------------------

fn layout_page(p: &PageInput<'_>, body: f32) -> PageLayout {
    let order = reading_order(p, body);
    let mut blocks: Vec<Block> = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let region = order[i].0;
        let mut j = i;
        while j < order.len() && order[j].0 == region {
            j += 1;
        }
        paragraphs(p, &order[i..j], region, &mut blocks);
        i = j;
    }
    PageLayout {
        blocks,
        page: p.page,
    }
}

fn paragraphs(p: &PageInput<'_>, items: &[(usize, Item)], region: usize, out: &mut Vec<Block>) {
    let lines: Vec<usize> = items
        .iter()
        .filter_map(|(_, it)| match it {
            Item::Line(l) => Some(*l),
            Item::Image(_) => None,
        })
        .collect();
    let left = lines
        .iter()
        .map(|&l| info(p.text, l).rect.x0)
        .fold(f64::INFINITY, f64::min);
    let right = lines
        .iter()
        .map(|&l| info(p.text, l).rect.x1)
        .fold(f64::NEG_INFINITY, f64::max);
    // The usual gap between consecutive lines of this region.
    let mut gaps: Vec<f64> = lines
        .windows(2)
        .map(|w| f64::from(info(p.text, w[1]).baseline - info(p.text, w[0]).baseline))
        .filter(|g| *g > 0.0)
        .collect();
    gaps.sort_by(f64::total_cmp);
    let usual = gaps.get(gaps.len() / 4).copied();
    // Justified text: most lines reach the right edge.
    let full = lines
        .iter()
        .filter(|&&l| right - info(p.text, l).rect.x1 < f64::from(info(p.text, l).size) * 0.6)
        .count();
    let justified = lines.len() >= 3 && full * 10 >= lines.len() * 6;

    let mut current: Option<Block> = None;
    let mut prev: Option<usize> = None;
    for (_, it) in items {
        match *it {
            Item::Image(k) => {
                if let Some(b) = current.take() {
                    out.push(b);
                }
                prev = None;
                out.push(Block {
                    kind: BlockKind::Image,
                    lines: Vec::new(),
                    rect: p.images[k],
                    region,
                    indented: false,
                });
            }
            Item::Line(l) => {
                let li = info(p.text, l);
                let size = f64::from(li.size.max(1.0));
                let indented = li.rect.x0 - left > size * 0.8;
                let new_block = match prev {
                    None => true,
                    Some(q) => {
                        let lq = info(p.text, q);
                        let step = f64::from(li.baseline - lq.baseline);
                        let wide = usual.is_some_and(|u| step > u * 1.18)
                            || step > f64::from(lq.size.max(li.size)) * 2.2;
                        let restyled = (li.size - lq.size).abs() > lq.size.min(li.size) * 0.08
                            || li.bold != lq.bold;
                        let indent_starts = indented && lq.rect.x0 - left < size * 0.5;
                        let short_before = justified && right - lq.rect.x1 > size * 2.0;
                        wide || restyled || indent_starts || short_before
                    }
                };
                if new_block && let Some(b) = current.take() {
                    out.push(b);
                }
                match current.as_mut() {
                    Some(b) => {
                        b.lines.push(l);
                        b.rect = b.rect.union(li.rect);
                    }
                    None => {
                        current = Some(Block {
                            kind: BlockKind::Paragraph,
                            lines: vec![l],
                            rect: li.rect,
                            region,
                            indented,
                        })
                    }
                }
                prev = Some(l);
            }
        }
    }
    if let Some(b) = current {
        out.push(b);
    }
}

// --- 3. kinds ----------------------------------------------------------------

fn block_text(text: &PageText, b: &Block) -> String {
    b.lines
        .iter()
        .map(|&l| line_text(text, l))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Digits of any script, so page numbers in Bangla or Devanagari count.
fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || c.is_ascii_digit() || ('০'..='৯').contains(&c) || ('०'..='९').contains(&c)
}

/// A running header's text with its page number taken out.
fn furniture_key(s: &str) -> String {
    s.chars()
        .filter(|c| !is_digit(*c) && !c.is_whitespace())
        .collect()
}

fn is_page_number(s: &str) -> bool {
    let t: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !"-–—|·.()[]".contains(*c))
        .collect();
    !t.is_empty()
        && t.chars().count() <= 6
        && (t.chars().all(is_digit) || t.chars().all(|c| "ivxlcdmIVXLCDM".contains(c)))
}

fn mark_furniture(pages: &[PageInput<'_>], out: &mut [PageLayout], body: f32) {
    // Candidates: single-line blocks in the top or bottom 10% of a page.
    let band = |layout: &PageLayout, b: &Block| {
        let h = layout.page.height().max(1.0);
        let top = (b.rect.y1 - layout.page.y0) / h < 0.12;
        let bottom = (b.rect.y0 - layout.page.y0) / h > 0.88;
        top || bottom
    };
    let keys: Vec<Vec<String>> = out
        .iter()
        .zip(pages)
        .map(|(layout, p)| {
            layout
                .blocks
                .iter()
                .filter(|b| b.lines.len() == 1 && band(layout, b))
                .map(|b| furniture_key(&block_text(p.text, b)))
                .filter(|k| !k.is_empty())
                .collect()
        })
        .collect();
    for (pi, (layout, p)) in out.iter_mut().zip(pages).enumerate() {
        let page_box = layout.page;
        let rects: Vec<Rect> = layout.blocks.iter().map(|b| b.rect).collect();
        for (bi, b) in layout.blocks.iter_mut().enumerate() {
            if b.kind == BlockKind::Image {
                continue;
            }
            let tagged = b.lines.iter().all(|&l| info(p.text, l).artifact);
            let in_band = b.lines.len() == 1 && {
                let h = page_box.height().max(1.0);
                (b.rect.y1 - page_box.y0) / h < 0.12 || (b.rect.y0 - page_box.y0) / h > 0.88
            };
            let text = block_text(p.text, b);
            let repeated = in_band && {
                let k = furniture_key(&text);
                !k.is_empty()
                    && (pi.saturating_sub(2)..(pi + 3).min(keys.len()))
                        .filter(|&q| q != pi)
                        .any(|q| keys[q].contains(&k))
            };
            // A page number is small or body-sized, alone on its line.
            let number =
                in_band && is_page_number(&text) && b.rect.height() < f64::from(body) * 2.5;
            // On a page with no neighbours to compare with, a short small
            // line at the very top or bottom, set apart from the text by more
            // than a line gap, is a running header or footer.
            let alone = pages.len() == 1 && in_band && {
                let h = page_box.height().max(1.0);
                let edge =
                    (b.rect.y1 - page_box.y0) / h < 0.08 || (b.rect.y0 - page_box.y0) / h > 0.92;
                let gap = rects
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != bi)
                    .map(|(_, r)| (r.y0 - b.rect.y1).max(b.rect.y0 - r.y1))
                    .fold(f64::INFINITY, f64::min);
                edge && block_size(p.text, b) < body * 0.9
                    && text.chars().count() <= 80
                    && gap >= f64::from(body)
            };
            if tagged || repeated || number || alone {
                b.kind = BlockKind::Furniture;
            }
        }
    }
}

fn block_size(text: &PageText, b: &Block) -> f32 {
    let mut s: Vec<f32> = b.lines.iter().map(|&l| info(text, l).size).collect();
    s.sort_by(f32::total_cmp);
    s.get(s.len() / 2).copied().unwrap_or(0.0)
}

fn mark_footnotes(pages: &[PageInput<'_>], out: &mut [PageLayout], body: f32) {
    for (layout, p) in out.iter_mut().zip(pages) {
        let h = layout.page.height().max(1.0);
        // Body text ends where the last body-sized paragraph ends.
        let body_bottom = layout
            .blocks
            .iter()
            .filter(|b| {
                b.kind == BlockKind::Paragraph && (block_size(p.text, b) - body).abs() < body * 0.1
            })
            .map(|b| b.rect.y1)
            .fold(f64::NEG_INFINITY, f64::max);
        for b in &mut layout.blocks {
            if b.kind != BlockKind::Paragraph {
                continue;
            }
            let small = block_size(p.text, b) < body * 0.88;
            let low = (b.rect.y0 - layout.page.y0) / h > 0.6 && b.rect.y0 >= body_bottom - 1.0;
            if small && low {
                b.kind = BlockKind::Footnote;
            }
        }
    }
}

fn mark_headings(pages: &[PageInput<'_>], out: &mut [PageLayout], body: f32) {
    let body_bold = {
        let (mut bold, mut all) = (0usize, 0usize);
        for p in pages {
            for (li, r) in p.text.line_info.iter().zip(&p.text.lines) {
                if (li.size - body).abs() < body * 0.1 {
                    all += r.len();
                    if li.bold {
                        bold += r.len();
                    }
                }
            }
        }
        bold * 2 > all
    };
    let mut sizes: Vec<f32> = Vec::new();
    let mut found: Vec<(usize, usize, f32)> = Vec::new();
    for (pi, (layout, p)) in out.iter_mut().zip(pages).enumerate() {
        for (bi, b) in layout.blocks.iter().enumerate() {
            if b.kind != BlockKind::Paragraph {
                continue;
            }
            let size = block_size(p.text, b);
            let text = block_text(p.text, b);
            let chars = text.chars().count();
            let bold = b.lines.iter().all(|&l| info(p.text, l).bold);
            let larger = size >= body * 1.1 && b.lines.len() <= 4;
            let bold_short = bold
                && !body_bold
                && b.lines.len() <= 2
                && chars <= 120
                && !text.trim_end().ends_with(['.', '।', '॥', ',', ';']);
            if larger || bold_short {
                let key = (size * 2.0).round() / 2.0;
                if !sizes.contains(&key) {
                    sizes.push(key);
                }
                found.push((pi, bi, key));
            }
        }
    }
    sizes.sort_by(|a, b| b.total_cmp(a));
    for (pi, bi, key) in found {
        let level = sizes
            .iter()
            .position(|s| *s == key)
            .map_or(6, |i| (i + 1).min(6)) as u8;
        out[pi].blocks[bi].kind = BlockKind::Heading(level);
    }
}

/// Byte ranges of a block's lines in the page text.
pub fn block_ranges(text: &PageText, b: &Block) -> Vec<Range<usize>> {
    b.lines
        .iter()
        .filter_map(|&l| text.lines.get(l).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page of lines: (x0, baseline, x1, size, bold, text).
    fn page(lines: &[(f64, f32, f64, f32, bool, &str)]) -> PageText {
        let mut t = PageText::default();
        for &(x0, base, x1, size, bold, s) in lines {
            if !t.text.is_empty() {
                t.text.push('\n');
            }
            let start = t.text.len();
            t.text.push_str(s);
            t.lines.push(start..t.text.len());
            t.line_info.push(LineInfo {
                rect: Rect::new(
                    x0,
                    f64::from(base - size * 0.8),
                    x1,
                    f64::from(base + size * 0.25),
                ),
                baseline: base,
                size,
                bold,
                italic: false,
                artifact: false,
            });
        }
        t
    }

    fn kinds(layout: &PageLayout, text: &PageText) -> Vec<(BlockKind, String)> {
        layout
            .blocks
            .iter()
            .map(|b| (b.kind, block_text(text, b)))
            .collect()
    }

    const A5: Rect = Rect::new(0.0, 0.0, 420.0, 595.0);

    #[test]
    fn two_columns_are_read_column_by_column() {
        // Left column lines a1..a3, right column b1..b3, on the same baselines,
        // drawn row by row (a1 b1 a2 b2 ...).
        let t = page(&[
            (40.0, 100.0, 200.0, 10.0, false, "a1"),
            (220.0, 100.0, 380.0, 10.0, false, "b1"),
            (40.0, 112.0, 200.0, 10.0, false, "a2"),
            (220.0, 112.0, 380.0, 10.0, false, "b2"),
            (40.0, 124.0, 120.0, 10.0, false, "a3"),
            (220.0, 124.0, 300.0, 10.0, false, "b3"),
        ]);
        let pages = [PageInput {
            text: &t,
            images: &[],
            page: A5,
        }];
        let l = &analyse(&pages)[0];
        let order: Vec<String> = l.blocks.iter().map(|b| block_text(&t, b)).collect();
        assert_eq!(order, ["a1 a2 a3", "b1 b2 b3"]);
    }

    #[test]
    fn heading_over_two_columns_comes_first() {
        let t = page(&[
            (40.0, 60.0, 380.0, 18.0, true, "Title"),
            (40.0, 100.0, 200.0, 10.0, false, "a1"),
            (220.0, 100.0, 380.0, 10.0, false, "b1"),
            (40.0, 112.0, 200.0, 10.0, false, "a2"),
            (220.0, 112.0, 380.0, 10.0, false, "b2"),
        ]);
        let pages = [PageInput {
            text: &t,
            images: &[],
            page: A5,
        }];
        let l = &analyse(&pages)[0];
        assert_eq!(
            kinds(l, &t),
            [
                (BlockKind::Heading(1), "Title".into()),
                (BlockKind::Paragraph, "a1 a2".into()),
                (BlockKind::Paragraph, "b1 b2".into())
            ]
        );
    }

    #[test]
    fn indents_and_gaps_start_paragraphs() {
        let t = page(&[
            (40.0, 100.0, 380.0, 10.0, false, "one"),
            (40.0, 112.0, 380.0, 10.0, false, "one more"),
            (40.0, 124.0, 200.0, 10.0, false, "end."),
            (55.0, 136.0, 380.0, 10.0, false, "two"),
            (40.0, 148.0, 380.0, 10.0, false, "two more"),
            (40.0, 160.0, 380.0, 10.0, false, "two end"),
            (40.0, 184.0, 380.0, 10.0, false, "three after a gap"),
            (40.0, 196.0, 380.0, 10.0, false, "three"),
        ]);
        let pages = [PageInput {
            text: &t,
            images: &[],
            page: A5,
        }];
        let l = &analyse(&pages)[0];
        let texts: Vec<String> = l.blocks.iter().map(|b| block_text(&t, b)).collect();
        assert_eq!(
            texts,
            [
                "one one more end.",
                "two two more two end",
                "three after a gap three"
            ]
        );
        assert!(l.blocks[1].indented);
    }

    #[test]
    fn running_headers_and_page_numbers_are_furniture() {
        let pages_text: Vec<PageText> = (1..=3)
            .map(|n| {
                page(&[
                    (150.0, 30.0, 270.0, 8.0, false, "নদীর ধারে · অক্ষর"),
                    (40.0, 100.0, 380.0, 10.0, false, "body text line"),
                    (40.0, 112.0, 380.0, 10.0, false, "more body text"),
                    (205.0, 575.0, 215.0, 9.0, false, ["১", "২", "৩"][n - 1]),
                ])
            })
            .collect();
        let pages: Vec<PageInput<'_>> = pages_text
            .iter()
            .map(|t| PageInput {
                text: t,
                images: &[],
                page: A5,
            })
            .collect();
        for (l, t) in analyse(&pages).iter().zip(&pages_text) {
            let k = kinds(l, t);
            assert_eq!(k[0].0, BlockKind::Furniture, "{k:?}");
            assert_eq!(k[1].0, BlockKind::Paragraph);
            assert_eq!(k[2].0, BlockKind::Furniture);
        }
    }

    #[test]
    fn small_text_at_the_bottom_is_a_footnote() {
        let t = page(&[
            (40.0, 100.0, 380.0, 10.0, false, "body"),
            (40.0, 112.0, 380.0, 10.0, false, "body"),
            (40.0, 124.0, 380.0, 10.0, false, "body"),
            (40.0, 520.0, 300.0, 8.0, false, "1 a note"),
        ]);
        let pages = [PageInput {
            text: &t,
            images: &[],
            page: A5,
        }];
        let l = &analyse(&pages)[0];
        assert_eq!(l.blocks.last().unwrap().kind, BlockKind::Footnote);
    }

    #[test]
    fn thousands_of_columns_do_not_overflow_the_stack() {
        // 20,000 one-glyph "lines" on one baseline, each 2 pt apart from the
        // next by more than the column gap: a hostile or broken page.
        let lines: Vec<(f64, f32, f64, f32, bool, &str)> = (0..20_000)
            .map(|i| {
                (
                    f64::from(i) * 20.0,
                    100.0,
                    f64::from(i) * 20.0 + 5.0,
                    10.0,
                    false,
                    "x",
                )
            })
            .collect();
        let t = page(&lines);
        let pages = [PageInput {
            text: &t,
            images: &[],
            page: Rect::new(0.0, 0.0, 400_000.0, 595.0),
        }];
        let l = &analyse(&pages)[0];
        let n: usize = l.blocks.iter().map(|b| b.lines.len()).sum();
        assert_eq!(n, 20_000);
    }

    #[test]
    fn one_page_furniture_needs_a_gap() {
        // A small line close above the text is a caption or a note, not a
        // running header; the same line far above it is one.
        let near = page(&[
            (40.0, 40.0, 200.0, 8.0, false, "a small note"),
            (40.0, 52.0, 380.0, 10.0, false, "body text"),
            (40.0, 64.0, 380.0, 10.0, false, "body text"),
        ]);
        let far = page(&[
            (40.0, 30.0, 200.0, 8.0, false, "a running header"),
            (40.0, 80.0, 380.0, 10.0, false, "body text"),
            (40.0, 92.0, 380.0, 10.0, false, "body text"),
        ]);
        for (t, furniture) in [(&near, false), (&far, true)] {
            let pages = [PageInput {
                text: t,
                images: &[],
                page: A5,
            }];
            let l = &analyse(&pages)[0];
            assert_eq!(l.blocks[0].kind == BlockKind::Furniture, furniture);
        }
    }

    #[test]
    fn page_numbers_in_any_script() {
        assert!(is_page_number("১২"));
        assert!(is_page_number("- 12 -"));
        assert!(is_page_number("xiv"));
        assert!(is_page_number("१२"));
        assert!(!is_page_number("অক্ষর"));
        assert!(!is_page_number("2026 সাল"));
    }
}
