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
//!
//! Before all that, [`tables`](crate::tables) finds tables and boxes: each
//! is one item of the reading order, so a table's cells are never taken for
//! columns, and a box's lines are read on their own, inside it.

use std::ops::Range;

use hayro::kurbo::Rect;

use crate::tables::{self, Found, Table};
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
    /// A table; [`Block::table`] holds its cells.
    Table,
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
    /// Inside a box (a card, a shaded or framed note).
    pub boxed: bool,
    /// Which box of the page, for a boxed block.
    pub box_index: usize,
    /// The cells of a [`BlockKind::Table`].
    pub table: Option<Table>,
}

/// The blocks of one page, in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLayout {
    pub blocks: Vec<Block>,
    /// Page box, in points.
    pub page: Rect,
}

/// One page's input: its text, its picture boxes, its rules and filled
/// boxes (see [`tables`]) and its size.
pub struct PageInput<'a> {
    pub text: &'a PageText,
    pub images: &'a [Rect],
    pub rules: &'a [Rect],
    pub fills: &'a [Rect],
    pub page: Rect,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Line(usize),
    Image(usize),
    Table(usize),
    Box(usize),
}

/// A page with its tables and boxes found.
struct Ctx<'p, 'a> {
    p: &'p PageInput<'a>,
    found: Found,
    /// Line spacing on this page per text size (half points): the
    /// smallest baseline step between consecutive lines of that size.
    steps: Vec<(i32, f64)>,
}

/// Analyses a run of pages (a chapter).
pub fn analyse(pages: &[PageInput<'_>]) -> Vec<PageLayout> {
    let body = body_size(pages);
    let mut out: Vec<PageLayout> = Vec::with_capacity(pages.len());
    for p in pages {
        // A one-row grid at the top of a page goes on a table that ended
        // the page before with as many columns.
        // (Furniture is not marked yet: the bottom band is skipped.)
        let continues = out.last().and_then(|l: &PageLayout| {
            let h = l.page.height().max(1.0);
            l.blocks
                .iter()
                .rev()
                .find(|b| (b.rect.y0 - l.page.y0) / h < 0.88)
                .and_then(|b| b.table.as_ref())
                .map(Table::columns)
        });
        out.push(layout_page(p, body, continues));
    }
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

fn rect_of(p: &Ctx<'_, '_>, item: Item) -> Rect {
    match item {
        Item::Line(i) => info(p.p.text, i).rect,
        Item::Image(i) => p.p.images[i],
        Item::Table(i) => p.found.tables[i].rect,
        Item::Box(i) => p.found.boxes[i].0,
    }
}

/// Items in reading order, each with its region number.
fn reading_order(p: &Ctx<'_, '_>, body: f32) -> Vec<(usize, Item)> {
    let mut taken = vec![false; p.p.text.lines.len()];
    for l in p.found.tables.iter().flat_map(Table::lines) {
        taken[l] = true;
    }
    for (_, ls) in &p.found.boxes {
        for &l in ls {
            taken[l] = true;
        }
    }
    // Text over a picture (labels drawn on an illustration) is part of it:
    // the picture block shows it.
    let on_picture = |i: usize| {
        let c = info(p.p.text, i).rect.center();
        p.p.images.iter().any(|r| r.contains(c))
    };
    let mut items: Vec<Item> = (0..p.p.text.lines.len())
        .filter(|&i| !taken[i] && !line_text(p.p.text, i).trim().is_empty() && !on_picture(i))
        .map(Item::Line)
        .collect();
    items.extend((0..p.p.images.len()).map(Item::Image));
    items.extend((0..p.found.tables.len()).map(Item::Table));
    items.extend((0..p.found.boxes.len()).map(Item::Box));
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
    p: &Ctx<'_, '_>,
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

fn band_top(p: &Ctx<'_, '_>, band: &[Item]) -> f64 {
    band.iter()
        .map(|&it| rect_of(p, it).y0)
        .fold(f64::INFINITY, f64::min)
}

/// The middle of the widest gap, at least `min` wide, that no item crosses.
fn vertical_gap(p: &Ctx<'_, '_>, items: &[Item], min: f64) -> Option<f64> {
    gap_interval(p, items, min).map(|(lo, hi)| (lo + hi) / 2.0)
}

/// The widest gap, at least `min` wide, that no item crosses: (left, right).
fn gap_interval(p: &Ctx<'_, '_>, items: &[Item], min: f64) -> Option<(f64, f64)> {
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

fn layout_page(p: &PageInput<'_>, body: f32, continues: Option<usize>) -> PageLayout {
    let lines: Vec<usize> = (0..p.text.lines.len())
        .filter(|&i| !line_text(p.text, i).trim().is_empty())
        .collect();
    let mut found = tables::find(p.text, &lines, p.rules, p.fills, body);
    if !found.one_row.is_empty() {
        // The row goes on the table when it comes first on the page (below
        // the running header) and has that table's columns.
        let mut taken = vec![false; p.text.lines.len()];
        for l in found
            .tables
            .iter()
            .chain(&found.one_row)
            .flat_map(Table::lines)
        {
            taken[l] = true;
        }
        let h = p.page.height().max(1.0);
        let first_text = lines
            .iter()
            .filter(|&&l| !taken[l])
            .map(|&l| info(p.text, l).rect.y0)
            .filter(|y| (y - p.page.y0) / h > 0.12)
            .fold(f64::INFINITY, f64::min);
        let first_table = found
            .tables
            .iter()
            .map(|t| t.rect.y0)
            .fold(f64::INFINITY, f64::min);
        let row = &found.one_row[0];
        let on_top = row.rect.y0 < first_text && row.rect.y0 < first_table;
        if found.one_row.len() == 1 && on_top && continues == Some(row.columns()) {
            found.one_row_as_tables();
        } else {
            found.one_row_as_boxes();
        }
    }
    let mut ctx = Ctx {
        p,
        found,
        steps: Vec::new(),
    };
    let order = reading_order(&ctx, body);
    // Line spacing over the page's running text: consecutive lines of each
    // region, regions kept apart.
    let mut regions: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (region, it) in &order {
        if let Item::Line(l) = it {
            regions.entry(*region).or_default().push(*l);
        }
    }
    let mut flat: Vec<usize> = Vec::new();
    for ls in regions.values() {
        flat.extend(ls);
        flat.push(usize::MAX);
    }
    ctx.steps = page_steps(p.text, &flat);
    let mut blocks: Vec<Block> = Vec::new();
    by_region(&ctx, &order, body, &mut blocks);
    PageLayout {
        blocks,
        page: p.page,
    }
}

/// Blocks of items in reading order, region by region.
fn by_region(ctx: &Ctx<'_, '_>, order: &[(usize, Item)], body: f32, blocks: &mut Vec<Block>) {
    let mut i = 0;
    while i < order.len() {
        let region = order[i].0;
        let mut j = i;
        while j < order.len() && order[j].0 == region {
            j += 1;
        }
        paragraphs(ctx, &order[i..j], region, body, blocks);
        i = j;
    }
}

/// Line spacing per text size (half points), over lists of lines (each a
/// region in reading order) separated by `usize::MAX`: the smallest step
/// between consecutive lines of one size, at least 0.9 of the size.
/// Paragraph gaps, headings and figures only ever make steps larger.
fn page_steps(text: &PageText, lines: &[usize]) -> Vec<(i32, f64)> {
    let mut out: Vec<(i32, f64)> = Vec::new();
    for w in lines.windows(2) {
        if w.contains(&usize::MAX) {
            continue;
        }
        let (a, b) = (info(text, w[0]), info(text, w[1]));
        let step = f64::from(b.baseline - a.baseline);
        if (a.size - b.size).abs() > a.size.min(b.size) * 0.05 || step < f64::from(a.size) * 0.9 {
            continue;
        }
        let key = (a.size * 2.0).round() as i32;
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, s)) => *s = s.min(step),
            None => out.push((key, step)),
        }
    }
    out
}

fn paragraphs(
    ctx: &Ctx<'_, '_>,
    items: &[(usize, Item)],
    region: usize,
    body: f32,
    out: &mut Vec<Block>,
) {
    let p = ctx.p;
    let lines: Vec<usize> = items
        .iter()
        .filter_map(|(_, it)| match it {
            Item::Line(l) => Some(*l),
            _ => None,
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
    // Line spacing for a line's size, from the whole page.
    let usual_for = |size: f32| {
        let key = (size * 2.0).round() as i32;
        ctx.steps.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
    };
    // Justified text: most lines reach the right edge.
    let full = lines
        .iter()
        .filter(|&&l| right - info(p.text, l).rect.x1 < f64::from(info(p.text, l).size) * 0.6)
        .count();
    let justified = lines.len() >= 3 && full * 10 >= lines.len() * 6;
    // Centred text (a card, a title page): lines share their centre, not
    // their left edge; length and indent say nothing about paragraphs.
    let centred = lines.len() >= 2 && {
        let mut mids: Vec<f64> = lines
            .iter()
            .map(|&l| info(p.text, l).rect.center().x)
            .collect();
        mids.sort_by(f64::total_cmp);
        let mid = mids[mids.len() / 2];
        let size = f64::from(body);
        let lefts_differ = lines
            .iter()
            .any(|&l| (info(p.text, l).rect.x0 - left).abs() > size);
        lefts_differ && mids.iter().all(|m| (m - mid).abs() < size * 0.6)
    };

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
                    boxed: false,
                    box_index: 0,
                    table: None,
                });
            }
            Item::Table(k) => {
                if let Some(b) = current.take() {
                    out.push(b);
                }
                prev = None;
                let t = &ctx.found.tables[k];
                out.push(Block {
                    kind: BlockKind::Table,
                    lines: t.lines(),
                    rect: t.rect,
                    region,
                    indented: false,
                    boxed: false,
                    box_index: 0,
                    table: Some(t.clone()),
                });
            }
            Item::Box(k) => {
                if let Some(b) = current.take() {
                    out.push(b);
                }
                prev = None;
                // The box's lines, read on their own.
                let (_, lines) = &ctx.found.boxes[k];
                let inner = Ctx {
                    p,
                    found: Found::default(),
                    steps: ctx.steps.clone(),
                };
                let mut order = Vec::new();
                let mut r = 0;
                let items = lines.iter().map(|&l| Item::Line(l)).collect();
                cut(&inner, items, body, 0, &mut r, &mut order);
                let start = out.len();
                by_region(&inner, &order, body, out);
                for b in &mut out[start..] {
                    b.boxed = true;
                    b.region = region;
                    b.box_index = k;
                }
            }
            Item::Line(l) => {
                let li = info(p.text, l);
                let size = f64::from(li.size.max(1.0));
                let indented = !centred && li.rect.x0 - left > size * 0.8;
                let new_block = match prev {
                    None => true,
                    Some(q) => {
                        let lq = info(p.text, q);
                        let step = f64::from(li.baseline - lq.baseline);
                        let wide = usual_for(li.size).is_some_and(|u| step > u * 1.18)
                            || step > f64::from(lq.size.max(li.size)) * 2.2;
                        let restyled = (li.size - lq.size).abs() > lq.size.min(li.size) * 0.08
                            || li.bold != lq.bold;
                        let indent_starts = indented && lq.rect.x0 - left < size * 0.5;
                        let short_before = justified && !centred && right - lq.rect.x1 > size * 2.0;
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
                            boxed: false,
                            box_index: 0,
                            table: None,
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
            if matches!(b.kind, BlockKind::Image | BlockKind::Table) || b.boxed {
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
            if b.kind != BlockKind::Paragraph || b.boxed {
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
    // The block after (pi, bi) in the chapter that is not furniture.
    let next_of = |pi: usize, bi: usize| {
        out.iter()
            .zip(pages)
            .enumerate()
            .skip(pi)
            .flat_map(|(qi, (l, p))| {
                l.blocks
                    .iter()
                    .enumerate()
                    .filter(move |(qb, _)| qi > pi || *qb > bi)
                    .map(move |(_, b)| (b, p))
            })
            .find(|(b, _)| b.kind != BlockKind::Furniture)
    };
    for (pi, (layout, p)) in out.iter().zip(pages).enumerate() {
        for (bi, b) in layout.blocks.iter().enumerate() {
            if b.kind != BlockKind::Paragraph {
                continue;
            }
            let size = block_size(p.text, b);
            // Inside a box only a much larger line is a heading: a card's
            // bold title is not one.
            if b.boxed && size < body * 1.5 {
                continue;
            }
            let text = block_text(p.text, b);
            let chars = text.chars().count();
            let bold = b.lines.iter().all(|&l| info(p.text, l).bold);
            // 1.08: LaTeX's \large over 11 pt text is 12 pt.
            let larger = size >= body * 1.08 && b.lines.len() <= 4;
            // A bold short line is a heading when body text follows it; a
            // lead-in ending in a colon ("Examples:") introduces what
            // follows and stays a paragraph.
            let body_follows = next_of(pi, bi).is_some_and(|(n, np)| {
                n.kind == BlockKind::Paragraph
                    && !n.boxed
                    && (block_size(np.text, n) - body).abs() < body * 0.1
                    && !n.lines.iter().all(|&l| info(np.text, l).bold)
            });
            let bold_short = bold
                && !body_bold
                && b.lines.len() <= 2
                && chars <= 120
                && !text.trim_end().ends_with(['.', '।', '॥', ',', ';', ':'])
                && body_follows;
            // A lead-in ("Difficulty level:") or a number is never one.
            let lead_in = text.trim_end().ends_with(':');
            let no_letters = !text.chars().any(char::is_alphabetic);
            if (larger || bold_short) && !lead_in && !no_letters {
                let key = (size * 2.0).round() / 2.0;
                if !sizes.contains(&key) {
                    sizes.push(key);
                }
                found.push((pi, bi, key));
            }
        }
    }
    // Three or more in a row, of one size, with nothing between them, are
    // labels (around a picture, in a word list), not headings.
    let order: Vec<(usize, usize)> = out
        .iter()
        .enumerate()
        .flat_map(|(pi, l)| {
            l.blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.kind != BlockKind::Furniture)
                .map(move |(bi, _)| (pi, bi))
        })
        .collect();
    let key_of = |pb: (usize, usize)| found.iter().find(|f| (f.0, f.1) == pb).map(|f| f.2);
    let mut labels: std::collections::HashSet<(usize, usize)> = Default::default();
    let mut run: Vec<(usize, usize)> = Vec::new();
    let mut run_key: Option<f32> = None;
    for &pb in order
        .iter()
        .chain(std::iter::once(&(usize::MAX, usize::MAX)))
    {
        let k = if pb.0 == usize::MAX { None } else { key_of(pb) };
        if k.is_some() && k == run_key {
            run.push(pb);
            continue;
        }
        if run.len() >= 3 {
            labels.extend(run.iter().copied());
        }
        run.clear();
        run_key = k;
        if k.is_some() {
            run.push(pb);
        }
    }
    found.retain(|f| !labels.contains(&(f.0, f.1)));
    sizes.retain(|s| found.iter().any(|f| f.2 == *s));
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
            rules: &[],
            fills: &[],
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
            rules: &[],
            fills: &[],
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
            rules: &[],
            fills: &[],
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
                rules: &[],
                fills: &[],
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
            rules: &[],
            fills: &[],
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
            rules: &[],
            fills: &[],
            page: Rect::new(0.0, 0.0, 400_000.0, 595.0),
        }];
        let l = &analyse(&pages)[0];
        let n: usize = l.blocks.iter().map(|b| b.lines.len()).sum();
        assert_eq!(n, 20_000);
    }

    #[test]
    fn centred_lines_stay_one_paragraph() {
        // A centred note: lines of different length share their centre;
        // a shorter line is not an indent, nor the end of a paragraph.
        let t = page(&[
            (
                40.0,
                100.0,
                380.0,
                10.0,
                false,
                "body text body text body text",
            ),
            (
                40.0,
                112.0,
                380.0,
                10.0,
                false,
                "body text body text body text",
            ),
            (
                90.0,
                136.0,
                330.0,
                10.0,
                false,
                "a centred sentence that runs",
            ),
            (150.0, 148.0, 270.0, 10.0, false, "onto a line"),
            (70.0, 160.0, 350.0, 10.0, false, "and a third, longer line."),
        ]);
        let pages = [PageInput {
            text: &t,
            images: &[],
            rules: &[],
            fills: &[],
            page: A5,
        }];
        let l = &analyse(&pages)[0];
        let texts: Vec<String> = l.blocks.iter().map(|b| block_text(&t, b)).collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
    }

    #[test]
    fn bold_lead_in_ending_in_a_colon_is_not_a_heading() {
        let t = page(&[
            (
                40.0,
                100.0,
                380.0,
                10.0,
                false,
                "body text body text body text",
            ),
            (40.0, 112.0, 200.0, 10.0, false, "more body text."),
            (40.0, 130.0, 160.0, 10.0, true, "Remember these words:"),
            (
                40.0,
                148.0,
                380.0,
                10.0,
                false,
                "body text body text body text",
            ),
            (40.0, 160.0, 200.0, 10.0, false, "more body text."),
            (40.0, 178.0, 160.0, 10.0, true, "A short bold heading"),
            (
                40.0,
                196.0,
                380.0,
                10.0,
                false,
                "body text body text body text",
            ),
            (40.0, 208.0, 200.0, 10.0, false, "more body text."),
        ]);
        let pages = [PageInput {
            text: &t,
            images: &[],
            rules: &[],
            fills: &[],
            page: A5,
        }];
        let k = kinds(&analyse(&pages)[0], &t);
        assert_eq!(k[1], (BlockKind::Paragraph, "Remember these words:".into()));
        assert_eq!(k[3], (BlockKind::Heading(1), "A short bold heading".into()));
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
                rules: &[],
                fills: &[],
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
