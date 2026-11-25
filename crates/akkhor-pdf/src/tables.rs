//! Tables and boxes on a page, found before reading order so that the gaps
//! between table cells are never taken for columns, and a box's text is
//! read on its own.
//!
//! Three kinds of table:
//!
//! - **ruled**: horizontal and vertical rules crossing into a grid of at
//!   least 2 by 2 cells (Word's "Table Grid", HTML borders, `|l|l|`). A grid
//!   of one row, or a single frame, is not a table but boxes (cards set side
//!   by side in a borderless row are the same thing drawn with fills).
//! - **horizontal rules only**: three or more rules of the same width
//!   (booktabs, row strokes); columns come from gaps every line between the
//!   rules leaves open.
//! - **unruled**: at least three rows whose cells line up in three or more
//!   columns, or in two where the first is narrow. Two columns of running
//!   text are equal in width, and stay columns.
//!
//! Boxes are filled rectangles (shaded cards, callouts) with text inside,
//! and the cells of one-row grids.
//!
//! Pure functions over line boxes and shapes, testable on hand-made pages.

use hayro::kurbo::Rect;

use crate::text::PageText;

/// A table: rows of cells, each cell its lines (indices into the page's
/// lines), top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub rect: Rect,
    pub rows: Vec<Vec<Vec<usize>>>,
    /// Every cell is a plain cell of the grid. False when cells were merged
    /// (a ruled grid with rules missing inside) or text runs across a
    /// column boundary: such a table is shown as a picture.
    pub regular: bool,
    /// For a one-row grid: each cell's box.
    pub cells: Vec<Rect>,
}

impl Table {
    pub fn columns(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    pub fn lines(&self) -> Vec<usize> {
        self.rows.iter().flatten().flatten().copied().collect()
    }
}

/// The tables and boxes of a page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub tables: Vec<Table>,
    /// Boxes and the lines inside each.
    pub boxes: Vec<(Rect, Vec<usize>)>,
    /// Ruled grids of a single row: cards side by side, or the last row of
    /// a table that the page break cut off (the caller knows which, from
    /// the page before). Their lines are taken.
    pub one_row: Vec<Table>,
}

impl Found {
    /// Takes the one-row grids as boxes, one per cell.
    pub fn one_row_as_boxes(&mut self) {
        for t in std::mem::take(&mut self.one_row) {
            for (c, lines) in t.rows.into_iter().flatten().enumerate() {
                if !lines.is_empty() {
                    let r = t.cells.get(c).copied().unwrap_or(t.rect);
                    self.boxes.push((r, lines));
                }
            }
        }
    }

    /// Takes the one-row grids as tables.
    pub fn one_row_as_tables(&mut self) {
        self.tables.append(&mut self.one_row);
    }
}

/// A horizontal (`pos` = y, `lo..hi` = x) or vertical (`pos` = x, `lo..hi`
/// = y) rule.
#[derive(Clone, Copy, Debug)]
struct Seg {
    pos: f64,
    lo: f64,
    hi: f64,
}

/// Joining tolerance, in points.
const NEAR: f64 = 2.5;

fn merge(mut segs: Vec<Seg>) -> Vec<Seg> {
    segs.sort_by(|a, b| a.pos.total_cmp(&b.pos).then(a.lo.total_cmp(&b.lo)));
    let mut out: Vec<Seg> = Vec::new();
    for s in segs {
        match out
            .iter_mut()
            .rev()
            .take(8)
            .find(|o| (o.pos - s.pos).abs() <= 1.5 && s.lo <= o.hi + NEAR && o.lo <= s.hi + NEAR)
        {
            Some(o) => {
                o.lo = o.lo.min(s.lo);
                o.hi = o.hi.max(s.hi);
            }
            None => out.push(s),
        }
    }
    out
}

/// Positions closer than `NEAR` are one.
fn cluster(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for x in v {
        if out.last().is_none_or(|l| x - l > NEAR) {
            out.push(x);
        }
    }
    out
}

fn find_root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Finds the tables and boxes among `lines` (indices of the page's
/// non-empty lines). `body` is the body text size.
pub fn find(text: &PageText, lines: &[usize], rules: &[Rect], fills: &[Rect], body: f32) -> Found {
    let body = f64::from(body.max(1.0));
    let rect = |l: usize| text.line_info[l].rect;
    let mut taken = vec![false; text.lines.len()];
    let mut found = Found::default();

    // --- rules -------------------------------------------------------------
    let (mut hs, mut vs) = (Vec::new(), Vec::new());
    for r in rules {
        if r.width() >= r.height() {
            hs.push(Seg {
                pos: r.center().y,
                lo: r.x0,
                hi: r.x1,
            });
        } else {
            vs.push(Seg {
                pos: r.center().x,
                lo: r.y0,
                hi: r.y1,
            });
        }
    }
    let (hs, vs) = (merge(hs), merge(vs));
    // Grids: horizontal and vertical rules that cross.
    let n = hs.len() + vs.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for (i, h) in hs.iter().enumerate() {
        for (j, v) in vs.iter().enumerate() {
            let touch = v.pos >= h.lo - NEAR
                && v.pos <= h.hi + NEAR
                && h.pos >= v.lo - NEAR
                && h.pos <= v.hi + NEAR;
            if touch {
                let (a, b) = (
                    find_root(&mut parent, i),
                    find_root(&mut parent, hs.len() + j),
                );
                parent[a] = b;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, (Vec<Seg>, Vec<Seg>)> = Default::default();
    for i in 0..n {
        let root = find_root(&mut parent, i);
        let g = groups.entry(root).or_default();
        if i < hs.len() {
            g.0.push(hs[i]);
        } else {
            g.1.push(vs[i - hs.len()]);
        }
    }
    let inside = |r: &Rect, l: usize| r.contains(rect(l).center());
    let mut loose_h: Vec<Seg> = Vec::new();
    for (gh, gv) in groups.into_values() {
        if gv.len() < 2 || gh.len() < 2 {
            if gv.is_empty() {
                loose_h.extend(gh);
            }
            continue;
        }
        let xs = cluster(gv.iter().map(|s| s.pos).collect());
        let ys = cluster(gh.iter().map(|s| s.pos).collect());
        if xs.len() < 2 || ys.len() < 2 {
            continue;
        }
        let bbox = Rect::new(xs[0], ys[0], xs[xs.len() - 1], ys[ys.len() - 1]);
        let within: Vec<usize> = lines
            .iter()
            .copied()
            .filter(|&l| !taken[l] && inside(&bbox, l))
            .collect();
        if within.is_empty() {
            continue;
        }
        let (rows, cols) = (ys.len() - 1, xs.len() - 1);
        let cell = |r: usize, c: usize| Rect::new(xs[c], ys[r], xs[c + 1], ys[r + 1]);
        if rows >= 2 && cols >= 2 {
            // Every inner edge of the grid drawn: no merged cells.
            let covered = |segs: &[Seg], pos: f64, lo: f64, hi: f64| {
                segs.iter()
                    .any(|s| (s.pos - pos).abs() <= NEAR && s.lo <= lo + NEAR && s.hi >= hi - NEAR)
            };
            let mut regular = true;
            for &x in xs.iter().take(cols).skip(1) {
                for r in 0..rows {
                    regular &= covered(&gv, x, ys[r], ys[r + 1]);
                }
            }
            for &y in ys.iter().take(rows).skip(1) {
                for c in 0..cols {
                    regular &= covered(&gh, y, xs[c], xs[c + 1]);
                }
            }
            let grid: Vec<Vec<Vec<usize>>> = (0..rows)
                .map(|r| {
                    (0..cols)
                        .map(|c| {
                            let mut ls: Vec<usize> = within
                                .iter()
                                .copied()
                                .filter(|&l| inside(&cell(r, c), l))
                                .collect();
                            ls.sort_by(|&a, &b| rect(a).y0.total_cmp(&rect(b).y0));
                            ls
                        })
                        .collect()
                })
                .filter(|row: &Vec<Vec<usize>>| row.iter().any(|c| !c.is_empty()))
                .collect();
            for &l in &within {
                taken[l] = true;
            }
            found.tables.push(Table {
                rect: bbox,
                rows: grid,
                regular,
                cells: Vec::new(),
            });
        } else if rows == 1 && cols >= 2 {
            // Cards in a row, or a table's last row: the caller decides.
            let cells: Vec<Rect> = (0..cols).map(|c| cell(0, c)).collect();
            let row: Vec<Vec<usize>> = cells
                .iter()
                .map(|r| {
                    let mut ls: Vec<usize> =
                        within.iter().copied().filter(|&l| inside(r, l)).collect();
                    ls.sort_by(|&a, &b| rect(a).y0.total_cmp(&rect(b).y0));
                    ls
                })
                .collect();
            for &l in &within {
                taken[l] = true;
            }
            found.one_row.push(Table {
                rect: bbox,
                rows: vec![row],
                regular: true,
                cells,
            });
        } else if rows == 1 {
            // A frame: a box if big enough to be one (not a number or a
            // label in a square, which stay text).
            if bbox.height() >= body * 2.0 && bbox.width() >= body * 4.0 {
                for &l in &within {
                    taken[l] = true;
                }
                found.boxes.push((bbox, within));
            }
        } else {
            // A frame with lines across it: a table if its text has
            // columns, else one box.
            let inner: Vec<Seg> = gh.clone();
            match text_table(text, &within, bbox, &inner, body) {
                Some(t) => {
                    for &l in &within {
                        taken[l] = true;
                    }
                    found.tables.push(t);
                }
                None => {
                    for &l in &within {
                        taken[l] = true;
                    }
                    found.boxes.push((bbox, within));
                }
            }
        }
    }

    // --- horizontal rules only --------------------------------------------
    loose_h.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    let mut used = vec![false; loose_h.len()];
    for i in 0..loose_h.len() {
        if used[i] {
            continue;
        }
        let first = loose_h[i];
        let mut stack = vec![first];
        let mut last = first.pos;
        for (j, s) in loose_h.iter().enumerate().skip(i + 1) {
            if used[j] {
                continue;
            }
            let same = (s.lo - first.lo).abs() <= 4.0 && (s.hi - first.hi).abs() <= 4.0;
            if same && s.pos - last <= body * 12.0 {
                stack.push(*s);
                used[j] = true;
                last = s.pos;
            }
        }
        if stack.len() < 3 {
            continue;
        }
        let bbox = Rect::new(first.lo, first.pos, first.hi, last);
        let within: Vec<usize> = lines
            .iter()
            .copied()
            .filter(|&l| !taken[l] && inside(&bbox, l))
            .collect();
        if let Some(t) = text_table(text, &within, bbox, &stack, body) {
            for &l in &within {
                taken[l] = true;
            }
            found.tables.push(t);
        }
    }

    // --- unruled ------------------------------------------------------------
    let free: Vec<usize> = lines.iter().copied().filter(|&l| !taken[l]).collect();
    for t in unruled(text, &free, body) {
        for l in t.lines() {
            taken[l] = true;
        }
        found.tables.push(t);
    }

    // --- filled boxes ---------------------------------------------------------
    let mut boxes: Vec<Rect> = Vec::new();
    for f in fills {
        // Cell shading inside a table, and the same box drawn twice.
        if found
            .tables
            .iter()
            .any(|t| t.rect.inflate(NEAR, NEAR).contains(f.center()))
        {
            continue;
        }
        let same = |b: &Rect| {
            let i = b.intersect(*f);
            i.area() > 0.8 * b.area().max(f.area())
        };
        if boxes.iter().any(same) {
            continue;
        }
        boxes.push(*f);
    }
    // Inner boxes first: a panel holding cards gives its lines to the cards.
    boxes.sort_by(|a, b| a.area().total_cmp(&b.area()));
    for b in boxes {
        let ls: Vec<usize> = lines
            .iter()
            .copied()
            .filter(|&l| !taken[l] && inside(&b, l))
            .collect();
        if ls.is_empty() {
            continue;
        }
        // A label or a number in a coloured square is not a box: a box
        // holds at least two lines' height and a few words' width.
        if b.height() < body * 2.0 || b.width() < body * 4.0 {
            continue;
        }
        // A highlight hugs its text; a box has room around it.
        let ink = ls
            .iter()
            .map(|&l| rect(l))
            .reduce(|a, r| a.union(r))
            .unwrap_or_default();
        if b.height() < ink.height() + 3.0 {
            continue;
        }
        for &l in &ls {
            taken[l] = true;
        }
        found.boxes.push((b, ls));
    }
    found
}

/// A table from the lines of a region whose rows are bounded by `rules`
/// (horizontal) and whose columns are wherever every line leaves a gap.
fn text_table(
    text: &PageText,
    within: &[usize],
    bbox: Rect,
    rules: &[Seg],
    body: f64,
) -> Option<Table> {
    let rect = |l: usize| text.line_info[l].rect;
    if within.len() < 2 {
        return None;
    }
    // Column boundaries: gaps no line crosses.
    let mut spans: Vec<(f64, f64)> = within.iter().map(|&l| (rect(l).x0, rect(l).x1)).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut starts = vec![spans[0].0];
    let mut reach = spans[0].1;
    for &(x0, x1) in &spans[1..] {
        if x0 - reach >= body * 0.6 {
            starts.push(x0);
        }
        reach = reach.max(x1);
    }
    if starts.len() < 2 {
        return None;
    }
    let column = |x: f64| starts.iter().rposition(|&s| x >= s - 1.0).unwrap_or(0);
    // Rows: between consecutive rules, split where a line starts in the
    // first column (rules only above and below the header, as booktabs).
    let mut ys: Vec<f64> = cluster(rules.iter().map(|s| s.pos).collect());
    ys.retain(|y| *y >= bbox.y0 - NEAR && *y <= bbox.y1 + NEAR);
    let mut sorted: Vec<usize> = within.to_vec();
    sorted.sort_by(|&a, &b| {
        text.line_info[a]
            .baseline
            .total_cmp(&text.line_info[b].baseline)
            .then(rect(a).x0.total_cmp(&rect(b).x0))
    });
    let mut rows: Vec<Vec<Vec<usize>>> = Vec::new();
    let mut band_of_last: Option<usize> = None;
    let mut last_first_col: Option<f32> = None;
    for &l in &sorted {
        let cy = rect(l).center().y;
        let band = ys.iter().filter(|&&y| y < cy).count();
        let base = text.line_info[l].baseline;
        let col = column(rect(l).x0);
        let new_band = band_of_last != Some(band);
        let new_row =
            new_band || (col == 0 && last_first_col.is_some_and(|b| (base - b).abs() > 0.5));
        if new_row || rows.is_empty() {
            rows.push(vec![Vec::new(); starts.len()]);
        }
        if col == 0 {
            last_first_col = Some(base);
        }
        band_of_last = Some(band);
        rows.last_mut().expect("pushed")[col].push(l);
    }
    // A line running over a column boundary: merged cells, or not a table.
    let crossing = within.iter().any(|&l| {
        let r = rect(l);
        starts
            .iter()
            .skip(1)
            .any(|&s| r.x0 < s - 2.0 && r.x1 > s + 2.0)
    });
    if rows.len() < 2 {
        return None;
    }
    Some(Table {
        rect: bbox,
        rows,
        regular: !crossing,
        cells: Vec::new(),
    })
}

/// Tables with no rules at all, from lines that line up in columns.
fn unruled(text: &PageText, lines: &[usize], body: f64) -> Vec<Table> {
    let info = |l: usize| &text.line_info[l];
    // Bands: lines on one baseline, left to right.
    let mut sorted: Vec<usize> = lines.to_vec();
    sorted.sort_by(|&a, &b| info(a).baseline.total_cmp(&info(b).baseline));
    let mut bands: Vec<Vec<usize>> = Vec::new();
    for l in sorted {
        let base = info(l).baseline;
        match bands.last_mut() {
            Some(b) if (info(b[0]).baseline - base).abs() <= info(l).size.max(1.0) * 0.3 => {
                b.push(l)
            }
            _ => bands.push(vec![l]),
        }
    }
    for b in &mut bands {
        b.sort_by(|&x, &y| info(x).rect.x0.total_cmp(&info(y).rect.x0));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < bands.len() {
        let head = &bands[i];
        let separate = head
            .windows(2)
            .all(|w| info(w[0]).rect.x1 + body * 0.6 <= info(w[1]).rect.x0);
        if head.len() < 2 || !separate {
            i += 1;
            continue;
        }
        let starts: Vec<f64> = head.iter().map(|&l| info(l).rect.x0).collect();
        // A line fits when it stays inside one column.
        let fits = |l: usize| {
            let r = info(l).rect;
            starts.iter().enumerate().any(|(k, &s)| {
                let end = starts.get(k + 1).map_or(f64::INFINITY, |n| n - 3.0);
                r.x0 >= s - 3.0 && r.x1 <= end
            })
        };
        let mut j = i + 1;
        let mut prev = info(head[0]).baseline;
        while j < bands.len() {
            let b = &bands[j];
            let step = f64::from(info(b[0]).baseline - prev);
            if step > body * 3.5 || !b.iter().all(|&l| fits(l)) {
                break;
            }
            prev = info(b[0]).baseline;
            j += 1;
        }
        let run = &bands[i..j];
        let multi = run.iter().filter(|b| b.len() >= 2).count();
        let right = run
            .iter()
            .flatten()
            .map(|&l| info(l).rect.x1)
            .fold(f64::NEG_INFINITY, f64::max);
        let narrow_first = (starts[1] - starts[0]) < 0.35 * (right - starts[0]);
        // Running text in columns: every column full of long lines.
        let column_of = |x: f64| starts.iter().rposition(|&s| x >= s - 3.0).unwrap_or(0);
        let full_columns = (0..starts.len()).all(|k| {
            let end = starts.get(k + 1).map_or(right, |n| *n);
            let width = (end - starts[k]).max(1.0);
            let mut ws: Vec<f64> = run
                .iter()
                .flatten()
                .filter(|&&l| column_of(info(l).rect.x0) == k)
                .map(|&l| info(l).rect.width() / width)
                .collect();
            ws.sort_by(f64::total_cmp);
            ws.get(ws.len() / 2).is_some_and(|m| *m > 0.7)
        });
        let table =
            multi >= 3 && (starts.len() >= 3 || narrow_first) && !(full_columns && run.len() >= 6);
        if !table {
            i += 1;
            continue;
        }
        let mut rows: Vec<Vec<Vec<usize>>> = Vec::new();
        let mut rect: Option<Rect> = None;
        for b in run {
            let starts_row = b.iter().any(|&l| column_of(info(l).rect.x0) == 0);
            if starts_row || rows.is_empty() {
                rows.push(vec![Vec::new(); starts.len()]);
            }
            for &l in b {
                rows.last_mut().expect("pushed")[column_of(info(l).rect.x0)].push(l);
                let r = info(l).rect;
                rect = Some(rect.map_or(r, |x| x.union(r)));
            }
        }
        out.push(Table {
            rect: rect.unwrap_or_default(),
            rows,
            regular: true,
            cells: Vec::new(),
        });
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::LineInfo;

    /// Lines: (x0, baseline, x1, text).
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

    fn cell_texts(t: &PageText, table: &Table) -> Vec<Vec<String>> {
        table
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|c| {
                        c.iter()
                            .map(|&l| t.text[t.lines[l].clone()].to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect()
            })
            .collect()
    }

    fn all(t: &PageText) -> Vec<usize> {
        (0..t.lines.len()).collect()
    }

    #[test]
    fn a_ruled_grid_is_a_table() {
        // Two columns, three rows, cell text placed in the grid.
        let t = page(&[
            (45.0, 112.0, 70.0, "word"),
            (125.0, 112.0, 160.0, "meaning"),
            (45.0, 132.0, 60.0, "nadi"),
            (125.0, 132.0, 190.0, "a river"),
            (125.0, 144.0, 170.0, "that flows"),
            (45.0, 162.0, 60.0, "ghat"),
            (125.0, 162.0, 200.0, "a landing"),
        ]);
        let mut rules = Vec::new();
        for y in [100.0, 120.0, 150.0, 170.0] {
            rules.push(Rect::new(40.0, y - 0.3, 300.0, y + 0.3));
        }
        for x in [40.0, 120.0, 300.0] {
            rules.push(Rect::new(x - 0.3, 100.0, x + 0.3, 170.0));
        }
        let f = find(&t, &all(&t), &rules, &[], 10.0);
        assert_eq!(f.tables.len(), 1);
        assert!(f.tables[0].regular);
        assert_eq!(
            cell_texts(&t, &f.tables[0]),
            [
                ["word", "meaning"],
                ["nadi", "a river that flows"],
                ["ghat", "a landing"]
            ]
        );
    }

    #[test]
    fn booktabs_rules_and_text_columns() {
        // Three rules: top, under the header, bottom; no vertical rules.
        let t = page(&[
            (45.0, 112.0, 70.0, "name"),
            (120.0, 112.0, 140.0, "math"),
            (160.0, 112.0, 190.0, "total"),
            (45.0, 132.0, 65.0, "rahim"),
            (125.0, 132.0, 140.0, "92"),
            (175.0, 132.0, 190.0, "325"),
            (45.0, 145.0, 62.0, "sumi"),
            (125.0, 145.0, 140.0, "88"),
            (175.0, 145.0, 190.0, "338"),
        ]);
        let rules: Vec<Rect> = [100.0, 120.0, 152.0]
            .iter()
            .map(|y| Rect::new(40.0, y - 0.4, 195.0, y + 0.4))
            .collect();
        let f = find(&t, &all(&t), &rules, &[], 10.0);
        assert_eq!(f.tables.len(), 1);
        assert_eq!(
            cell_texts(&t, &f.tables[0]),
            [
                ["name", "math", "total"],
                ["rahim", "92", "325"],
                ["sumi", "88", "338"]
            ]
        );
    }

    #[test]
    fn unruled_columns_line_up() {
        let t = page(&[
            (45.0, 112.0, 70.0, "word"),
            (100.0, 112.0, 140.0, "meaning"),
            (200.0, 112.0, 240.0, "example"),
            (45.0, 128.0, 70.0, "kolsi"),
            (100.0, 128.0, 170.0, "a clay pot"),
            (200.0, 128.0, 380.0, "girls carry the pot to"),
            (200.0, 140.0, 260.0, "the pond"),
            (45.0, 156.0, 70.0, "gamcha"),
            (100.0, 156.0, 170.0, "a towel"),
            (200.0, 156.0, 380.0, "the farmer ties rice in"),
        ]);
        let f = find(&t, &all(&t), &[], &[], 10.0);
        assert_eq!(f.tables.len(), 1);
        assert_eq!(
            cell_texts(&t, &f.tables[0]),
            [
                ["word", "meaning", "example"],
                ["kolsi", "a clay pot", "girls carry the pot to the pond"],
                ["gamcha", "a towel", "the farmer ties rice in"]
            ]
        );
    }

    #[test]
    fn two_columns_of_text_are_not_a_table() {
        let mut lines = Vec::new();
        for k in 0..8 {
            let base = 100.0 + 12.0 * k as f32;
            lines.push((40.0, base, 200.0, "left column text"));
            lines.push((220.0, base, 380.0, "right column text"));
        }
        let t = page(&lines);
        let f = find(&t, &all(&t), &[], &[], 10.0);
        assert!(f.tables.is_empty());
    }

    #[test]
    fn filled_boxes_and_one_row_grids_are_boxes() {
        let t = page(&[
            (60.0, 112.0, 100.0, "card one"),
            (160.0, 112.0, 200.0, "card two"),
            (60.0, 200.0, 300.0, "a shaded note"),
            (40.0, 300.0, 380.0, "body text, highlighted"),
        ]);
        // One-row grid of two cells.
        let mut rules = Vec::new();
        for y in [100.0, 120.0] {
            rules.push(Rect::new(40.0, y - 0.3, 260.0, y + 0.3));
        }
        for x in [40.0, 150.0, 260.0] {
            rules.push(Rect::new(x - 0.3, 100.0, x + 0.3, 120.0));
        }
        let fills = [
            Rect::new(50.0, 185.0, 320.0, 215.0),
            // A highlight hugging its line is not a box.
            Rect::new(39.0, 291.0, 381.0, 303.0),
        ];
        let mut f = find(&t, &all(&t), &rules, &fills, 10.0);
        assert!(f.tables.is_empty());
        f.one_row_as_boxes();
        let boxed: Vec<Vec<usize>> = f.boxes.iter().map(|(_, l)| l.clone()).collect();
        let mut boxed = boxed;
        boxed.sort();
        assert_eq!(boxed, [vec![0], vec![1], vec![2]]);
    }
}
