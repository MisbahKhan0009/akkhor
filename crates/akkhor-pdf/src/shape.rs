//! Coarse glyph shapes, for glyphs that can only be recognised by how they
//! look: Type 3 glyphs (drawing procedures, no font program, no text), such
//! as the synthetic bold Chrome draws when a font has no bold face. Their
//! outlines are the regular glyph's outline made thicker, so exact outline
//! matching cannot find them; a low-resolution picture of the shape can.
//!
//! A shape is a 64 × 32 bitmap of the em square around the glyph origin
//! (x from −0.5 to 1.5 em, y from −0.5 to 1.1 em, in 1000-unit ems, y up),
//! filled with the nonzero rule. Two shapes compare by overlap (Jaccard).

use hayro::kurbo::{BezPath, PathEl, Point};

/// Columns and rows of a shape bitmap.
const COLS: usize = 64;
const ROWS: usize = 32;
const X0: f64 = -500.0;
const X1: f64 = 1500.0;
const Y0: f64 = -500.0;
const Y1: f64 = 1100.0;

/// A glyph shape: one 64-bit row per scanline, bottom to top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Shape(pub [u64; ROWS]);

impl Shape {
    /// The shape of `path`, given in 1000-unit ems with y up.
    pub fn of(path: &BezPath) -> Self {
        let mut edges: Vec<(Point, Point)> = Vec::new();
        let mut start = Point::ZERO;
        let mut last = Point::ZERO;
        hayro::kurbo::flatten(path.iter(), 4.0, |el| match el {
            PathEl::MoveTo(p) => {
                if last != start {
                    edges.push((last, start));
                }
                start = p;
                last = p;
            }
            PathEl::LineTo(p) => {
                edges.push((last, p));
                last = p;
            }
            PathEl::ClosePath => {
                if last != start {
                    edges.push((last, start));
                }
                last = start;
            }
            _ => {}
        });
        if last != start {
            edges.push((last, start));
        }
        let mut rows = [0u64; ROWS];
        let cell_w = (X1 - X0) / COLS as f64;
        let cell_h = (Y1 - Y0) / ROWS as f64;
        for (r, row) in rows.iter_mut().enumerate() {
            let y = Y0 + (r as f64 + 0.5) * cell_h;
            // Crossings of the scanline, with the winding direction.
            let mut xs: Vec<(f64, i32)> = edges
                .iter()
                .filter_map(|&(a, b)| {
                    let (lo, hi, dir) = if a.y < b.y { (a, b, 1) } else { (b, a, -1) };
                    (lo.y <= y && y < hi.y).then(|| {
                        let t = (y - lo.y) / (hi.y - lo.y);
                        (lo.x + t * (hi.x - lo.x), dir)
                    })
                })
                .collect();
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for w in xs.windows(2) {
                winding += w[0].1;
                if winding != 0 {
                    for c in 0..COLS {
                        let x = X0 + (c as f64 + 0.5) * cell_w;
                        if w[0].0 <= x && x < w[1].0 {
                            *row |= 1 << c;
                        }
                    }
                }
            }
        }
        Shape(rows)
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|r| *r == 0)
    }

    /// The shape grown by one cell in every direction.
    pub fn dilated(&self) -> Shape {
        let mut out = [0u64; ROWS];
        for (r, row) in out.iter_mut().enumerate() {
            for dr in [-1i32, 0, 1] {
                let rr = r as i32 + dr;
                if (0..ROWS as i32).contains(&rr) {
                    let v = self.0[rr as usize];
                    *row |= v | (v << 1) | (v >> 1);
                }
            }
        }
        Shape(out)
    }

    /// How well a bolder copy `bold` matches this (regular) shape: the
    /// share of this shape's cells that `bold` covers, and the share of
    /// `bold`'s cells within this shape grown by a cell, whichever is
    /// lower. Thickening keeps both near 1; a different glyph does not.
    pub fn bold_match(&self, bold: &Shape, grown: &Shape) -> f32 {
        let (mut mine, mut covered, mut theirs, mut inside) = (0u32, 0u32, 0u32, 0u32);
        for ((a, b), g) in self.0.iter().zip(&bold.0).zip(&grown.0) {
            mine += a.count_ones();
            covered += (a & b).count_ones();
            theirs += b.count_ones();
            inside += (b & g).count_ones();
        }
        if mine == 0 || theirs == 0 {
            return 0.0;
        }
        (covered as f32 / mine as f32).min(inside as f32 / theirs as f32)
    }

    /// Cells inked.
    pub fn ink(&self) -> u32 {
        self.0.iter().map(|r| r.count_ones()).sum()
    }

    /// Overlap with `other`: shared cells over cells in either.
    pub fn similarity(&self, other: &Shape) -> f32 {
        let (mut both, mut either) = (0u32, 0u32);
        for (a, b) in self.0.iter().zip(&other.0) {
            both += (a & b).count_ones();
            either += (a | b).count_ones();
        }
        if either == 0 {
            return 0.0;
        }
        both as f32 / either as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro::kurbo::{Rect, Shape as _};

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    #[test]
    fn thicker_is_still_close_and_different_is_far() {
        let bar = Shape::of(&rect(100.0, 0.0, 200.0, 700.0));
        let bold_bar = Shape::of(&rect(80.0, -20.0, 220.0, 720.0));
        let dot = Shape::of(&rect(100.0, 0.0, 200.0, 100.0));
        assert!(!bar.is_empty());
        assert!(
            bar.similarity(&bold_bar) > 0.6,
            "{}",
            bar.similarity(&bold_bar)
        );
        assert!(bar.similarity(&dot) < 0.3);
        assert_eq!(bar.similarity(&bar), 1.0);
    }

    #[test]
    fn holes_are_empty() {
        // An outer square with an inner square drawn the other way round.
        let mut p = rect(0.0, 0.0, 800.0, 800.0);
        let mut inner = BezPath::new();
        inner.move_to((200.0, 200.0));
        inner.line_to((200.0, 600.0));
        inner.line_to((600.0, 600.0));
        inner.line_to((600.0, 200.0));
        inner.close_path();
        p.extend(inner);
        let s = Shape::of(&p);
        let solid = Shape::of(&rect(0.0, 0.0, 800.0, 800.0));
        assert!(s.similarity(&solid) < 0.85);
    }
}
