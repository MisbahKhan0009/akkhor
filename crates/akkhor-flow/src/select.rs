//! Hit-testing, selection and copy.
//!
//! Every position these functions return sits on an orthographic syllable
//! boundary, so a tap, a long-press or a drag handle can never land inside a
//! conjunct such as ক্ষ, ন্ত্র or द्ध.

use crate::layout::{Line, Page};
use crate::model::{Document, Position};
use crate::script::syllable_starts;

/// A rectangle in page pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Syllable boundaries on a line, as block byte offsets, including the end.
fn boundaries(doc: &Document, line: &Line) -> Vec<usize> {
    let Some(block) = doc.blocks.get(line.block) else {
        return vec![line.range.start, line.range.end];
    };
    let text = &block.text[line.range.clone()];
    let mut out: Vec<usize> = syllable_starts(text)
        .into_iter()
        .map(|o| o + line.range.start)
        .collect();
    out.push(line.range.end);
    out.dedup();
    out
}

/// The position under (x, y) on `page`, snapped to the nearest syllable
/// boundary. Points above or below the text pick the nearest line.
pub fn hit_test(doc: &Document, page: &Page, x: f32, y: f32) -> Option<Position> {
    let line = page.lines.iter().min_by(|a, b| {
        let da = distance(y, a.top, a.bottom);
        let db = distance(y, b.top, b.bottom);
        da.total_cmp(&db)
    })?;
    let offset = boundaries(doc, line).into_iter().min_by(|&a, &b| {
        (line.x_of(a) - x)
            .abs()
            .total_cmp(&(line.x_of(b) - x).abs())
    })?;
    Some(Position::new(line.block, offset))
}

fn distance(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo - v
    } else if v > hi {
        v - hi
    } else {
        0.0
    }
}

fn is_word_break(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '।' | '॥'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '?'
                | '"'
                | '\''
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '«'
                | '»'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '—'
                | '–'
        )
}

/// The word around `pos` (for long-press selection). Returns an empty range
/// at `pos` when it sits on a space or punctuation.
pub fn word_at(doc: &Document, pos: Position) -> (Position, Position) {
    let Some(block) = doc.blocks.get(pos.block) else {
        return (pos, pos);
    };
    let text = &block.text;
    let at = pos.offset.min(text.len());
    let start = text[..at]
        .char_indices()
        .rev()
        .find(|&(_, c)| is_word_break(c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[at..]
        .char_indices()
        .find(|&(_, c)| is_word_break(c))
        .map_or(text.len(), |(i, _)| at + i);
    if start >= end {
        return (pos, pos);
    }
    (
        Position::new(pos.block, start),
        Position::new(pos.block, end),
    )
}

/// Highlight rectangles for the text from `start` to `end` on `page`, one per
/// line piece. Use the same call for selections and saved highlights.
pub fn selection_rects(page: &Page, start: Position, end: Position) -> Vec<Rect> {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let mut out = Vec::new();
    for line in &page.lines {
        let line_start = Position::new(line.block, line.range.start);
        let line_end = Position::new(line.block, line.range.end);
        if end <= line_start || start >= line_end {
            continue;
        }
        let a = if start > line_start {
            start.offset
        } else {
            line.range.start
        };
        let b = if end < line_end {
            end.offset
        } else {
            line.range.end
        };
        let (x1, x2) = (line.x_of(a), line.x_of(b));
        if x2 > x1 {
            out.push(Rect {
                x: x1,
                y: line.top,
                width: x2 - x1,
                height: line.bottom - line.top,
            });
        }
    }
    out
}

/// The text between two positions, for copying. Blocks are separated by a
/// blank line.
pub fn text_between(doc: &Document, start: Position, end: Position) -> String {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let mut out = String::new();
    for b in start.block..=end.block.min(doc.blocks.len().saturating_sub(1)) {
        let Some(block) = doc.blocks.get(b) else {
            break;
        };
        let text = &block.text;
        let from = if b == start.block {
            start.offset.min(text.len())
        } else {
            0
        };
        let to = if b == end.block {
            end.offset.min(text.len())
        } else {
            text.len()
        };
        if from < to && text.is_char_boundary(from) && text.is_char_boundary(to) {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&text[from..to]);
        }
    }
    out
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;
    use crate::fonts::FontSet;
    use crate::layout::{LayoutOptions, paginate};

    fn setup(text: &str) -> (Document, Vec<Page>) {
        let fonts = FontSet::bundled();
        let doc = Document::from_plain_text(text);
        let pages = paginate(&doc, &LayoutOptions::default(), &fonts);
        (doc, pages)
    }

    #[test]
    fn taps_never_land_inside_a_conjunct() {
        for text in ["ক্ষমা মন্ত্রী উজ্জ্বল", "क्षत्रिय शुद्ध राष्ट्र", "ક્ષમા ಕ್ಷಮೆ క్షమ ക്ഷമ"]
        {
            let (doc, pages) = setup(text);
            let starts = syllable_starts(&doc.blocks[0].text);
            let line = &pages[0].lines[0];
            let mut x = line.left - 5.0;
            while x < line.right + 5.0 {
                let p = hit_test(&doc, &pages[0], x, line.baseline).unwrap();
                assert!(
                    p.offset == doc.blocks[0].text.len() || starts.contains(&p.offset),
                    "{text}: tap at {x} gave offset {} inside a syllable",
                    p.offset
                );
                x += 1.5;
            }
        }
    }

    #[test]
    fn taps_move_left_to_right() {
        let (doc, pages) = setup("আমার সোনার বাংলা");
        let line = &pages[0].lines[0];
        let a = hit_test(&doc, &pages[0], line.left, line.baseline).unwrap();
        let b = hit_test(&doc, &pages[0], line.right, line.baseline).unwrap();
        assert_eq!(a.offset, 0);
        assert_eq!(b.offset, doc.blocks[0].text.len());
    }

    #[test]
    fn words_and_copy() {
        let (doc, pages) = setup("আমি বাংলায় গান গাই। हम हिन्दी पढ़ते हैं।");
        let text = &doc.blocks[0].text;
        let inside = text.find("লায়").unwrap();
        let (s, e) = word_at(&doc, Position::new(0, inside));
        assert_eq!(text_between(&doc, s, e), "বাংলায়");
        let inside = text.find("न्दी").unwrap();
        let (s, e) = word_at(&doc, Position::new(0, inside));
        assert_eq!(text_between(&doc, s, e), "हिन्दी");
        let rects = selection_rects(&pages[0], s, e);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].width > 10.0);
    }

    #[test]
    fn selection_spans_lines() {
        let (doc, pages) = setup(&"আমার সোনার বাংলা, আমি তোমায় ভালোবাসি। ".repeat(6));
        let end = Position::new(0, doc.blocks[0].text.len());
        let rects = selection_rects(&pages[0], Position::new(0, 0), end);
        assert_eq!(rects.len(), pages[0].lines.len());
    }
}
