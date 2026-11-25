//! The text layer of a page: glyphs turned into lines of logical text, with
//! every byte of the text pointing back to a box on the page.

use std::ops::Range;

use hayro::kurbo::Rect;

use akkhor_flow::script::{IndicCat, indic_cat};

use crate::glyphs::{PageGlyphs, PdfGlyph};
use crate::recover::{Role, reph_split, starts_with_letter};

/// A piece of page text and the box it occupies.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    /// Byte range in [`PageText::text`].
    pub range: Range<usize>,
    /// Box on the page, in points (top-left origin).
    pub rect: Rect,
    /// Index into [`PageText::lines`].
    pub line: usize,
}

/// The readable text of one page.
#[derive(Clone, Debug, Default)]
pub struct PageText {
    /// Lines separated by `'\n'`, paragraphs by a blank line.
    pub text: String,
    /// Boxes for the text, in text order.
    pub spans: Vec<TextSpan>,
    /// Byte range of each line in `text`.
    pub lines: Vec<Range<usize>>,
}

impl PageText {
    /// Boxes covering `range` of the text, merged per line: what a
    /// selection or highlight should paint.
    pub fn rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut out: Vec<(usize, Rect)> = Vec::new();
        for s in &self.spans {
            if s.range.end <= range.start || s.range.start >= range.end {
                continue;
            }
            match out.last_mut() {
                Some((line, r)) if *line == s.line => *r = r.union(s.rect),
                _ => out.push((s.line, s.rect)),
            }
        }
        out.into_iter().map(|(_, r)| r).collect()
    }

    /// The byte offset of the text nearest to page point (x, y), for
    /// hit-testing, on a syllable boundary (a selection never splits a
    /// conjunct). `None` on an empty page.
    pub fn offset_at(&self, x: f64, y: f64) -> Option<usize> {
        let dist = |r: &Rect| {
            let dx = if x < r.x0 {
                r.x0 - x
            } else if x > r.x1 {
                x - r.x1
            } else {
                0.0
            };
            let dy = if y < r.y0 {
                r.y0 - y
            } else if y > r.y1 {
                y - r.y1
            } else {
                0.0
            };
            dy * 4.0 + dx
        };
        let s = self
            .spans
            .iter()
            .min_by(|a, b| dist(&a.rect).total_cmp(&dist(&b.rect)))?;
        // Before or after the span, by which half was hit.
        let at = if x < (s.rect.x0 + s.rect.x1) / 2.0 {
            s.range.start
        } else {
            s.range.end
        };
        Some(self.snap(at))
    }
    /// `offset` moved back to the start of its syllable.
    pub fn snap(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        let line = self
            .lines
            .iter()
            .find(|l| l.start <= offset && offset <= l.end)
            .cloned()
            .unwrap_or(0..self.text.len());
        if offset == line.end {
            return offset;
        }
        let starts = akkhor_flow::script::syllable_starts(&self.text[line.clone()]);
        let rel = offset - line.start;
        line.start
            + starts
                .iter()
                .rev()
                .find(|&&s| s <= rel)
                .copied()
                .unwrap_or(0)
    }

    /// The word around `offset`: the run of text between spaces and
    /// punctuation that contains it.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let is_break = |c: char| {
            c.is_whitespace()
                || matches!(c, '।' | '॥' | ',' | ';' | ':' | '!' | '?' | '"' | '(' | ')')
        };
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .find(|&(_, c)| is_break(c))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let end = self.text[offset..]
            .char_indices()
            .find(|&(_, c)| is_break(c))
            .map_or(self.text.len(), |(i, _)| offset + i);
        start..end.max(start)
    }

    /// Every occurrence of `query` on the page (Indic-aware folding, whole
    /// syllables only), as byte ranges.
    pub fn find(&self, query: &str) -> Vec<Range<usize>> {
        let q = akkhor_flow::search::fold(query);
        akkhor_flow::search::find_in_text(&self.text, &q)
            .into_iter()
            .map(|(s, e)| s..e)
            .collect()
    }
}

/// What one glyph contributes to the text, decided by the caller.
pub(crate) struct GlyphPiece<'g> {
    pub glyph: &'g PdfGlyph,
    pub text: String,
    pub role: Role,
    /// Box covering this piece (grows when pieces merge).
    pub rect: Rect,
}

impl<'g> GlyphPiece<'g> {
    pub(crate) fn new(glyph: &'g PdfGlyph, text: String, role: Role) -> Self {
        Self {
            glyph,
            text,
            role,
            rect: glyph.rect,
        }
    }
}

/// Groups glyph pieces into lines and paragraphs. Glyphs are taken in
/// drawing order, which for text is reading order; a new line starts when
/// the baseline moves or the pen jumps back to the left.
pub(crate) fn assemble(
    page: &PageGlyphs,
    mut word_pieces: impl for<'g> FnMut(&[&'g PdfGlyph]) -> Vec<GlyphPiece<'g>>,
    mut finish_line: impl FnMut(Vec<GlyphPiece<'_>>) -> Vec<GlyphPiece<'_>>,
) -> PageText {
    // Split into lines.
    let mut lines: Vec<Vec<&PdfGlyph>> = Vec::new();
    for g in &page.glyphs {
        let new_line = match lines.last().and_then(|l| l.last()) {
            None => true,
            Some(prev) => {
                let size = prev.size.max(g.size).max(1.0);
                let baseline_moved = (g.origin.1 - prev.origin.1).abs() > size * 0.5;
                let jumped_back = g.origin.0 + size < prev.origin.0;
                baseline_moved || jumped_back
            }
        };
        if new_line {
            lines.push(Vec::new());
        }
        lines.last_mut().expect("pushed").push(g);
    }

    let mut out = PageText::default();
    let mut prev_line: Option<(f32, f32, f32)> = None; // baseline, size, left
    for line in lines {
        // Words first, in drawing order: a visible gap between glyphs is a
        // space the PDF did not draw. Reordering then stays inside a word.
        let mut spans: Vec<Range<usize>> = Vec::new();
        let mut pen_end: Option<f32> = None;
        for (i, g) in line.iter().enumerate() {
            let gap = pen_end.is_some_and(|end| g.origin.0 - end > g.size * 0.22);
            // Marks are positioned back over their base: the pen is at the
            // furthest point reached, not where the last glyph ended.
            let end = g.origin.0 + g.advance;
            pen_end = Some(pen_end.map_or(end, |e: f32| e.max(end)));
            match spans.last_mut() {
                Some(w) if !gap => w.end = i + 1,
                _ => spans.push(i..i + 1),
            }
        }
        let words: Vec<Vec<GlyphPiece<'_>>> =
            spans.into_iter().map(|r| word_pieces(&line[r])).collect();
        let words: Vec<Vec<GlyphPiece<'_>>> = words.into_iter().map(&mut finish_line).collect();
        if words.iter().flatten().all(|p| p.text.trim().is_empty()) {
            continue;
        }
        let first = line[0];
        if let Some((base, size, _)) = prev_line {
            let gap = first.origin.1 - base;
            out.text.push('\n');
            if gap > size.max(first.size) * 2.2 {
                out.text.push('\n');
            }
        }
        let line_index = out.lines.len();
        let line_start = out.text.len();
        for word in &words {
            if out.text.len() > line_start && !out.text.ends_with([' ', '\n']) {
                out.text.push(' ');
            }
            for p in word {
                if p.text.is_empty() {
                    continue;
                }
                if p.text.starts_with(' ') && out.text.ends_with(' ') {
                    out.text.pop();
                }
                let start = out.text.len();
                out.text.push_str(&p.text);
                out.spans.push(TextSpan {
                    range: start..out.text.len(),
                    rect: p.rect,
                    line: line_index,
                });
            }
        }
        while out.text.ends_with(' ') {
            out.text.pop();
        }
        out.lines.push(line_start..out.text.len());
        prev_line = Some((first.origin.1, first.size, first.origin.0));
    }
    out
}

/// Whether text from a PDF's own text layer can be trusted for a glyph.
pub(crate) fn usable(s: &str) -> bool {
    !s.is_empty()
        && !s.chars().any(|c| {
            c == '\0' || c == '\u{FFFD}' || ('\u{E000}'..='\u{F8FF}').contains(&c) || c.is_control()
        })
}

fn ends_with_virama(text: &str) -> bool {
    text.chars()
        .next_back()
        .is_some_and(|c| matches!(indic_cat(c), Some((_, IndicCat::Virama))))
}

/// Whether a piece continues the consonant cluster before it: it starts
/// with a virama (a phala or below-base form), a nukta or a joiner.
fn continues_cluster(text: &str) -> bool {
    text.chars().next().is_some_and(|c| {
        matches!(c, '\u{200C}' | '\u{200D}')
            || matches!(indic_cat(c), Some((_, IndicCat::Virama | IndicCat::Nukta)))
    })
}

fn is_ra_virama(text: &str) -> bool {
    let mut c = text.chars();
    matches!(
        (c.next(), c.next(), c.next()),
        (Some('র' | 'र' | 'ਰ' | 'ર' | 'ର' | 'ర' | 'ರ' | 'ര'), Some(v), None)
            if matches!(indic_cat(v), Some((_, IndicCat::Virama)))
    )
}

/// Two-part vowels, written as their two halves after reordering. The
/// second half may carry more marks (Devanagari ें after ा makes ों).
fn compose_split_vowel(first: &str, second: &str) -> Option<String> {
    let mut rest = second.chars();
    let head = rest.next()?;
    let joined = match (first, head) {
        ("\u{09C7}", '\u{09BE}') => '\u{09CB}', // ে া → ো
        ("\u{09C7}", '\u{09D7}') => '\u{09CC}', // ে ৗ → ৌ
        ("\u{093E}", '\u{0947}') => '\u{094B}', // ा े → ो (drawn as ा + े)
        ("\u{093E}", '\u{0948}') => '\u{094C}', // ा ै → ौ
        ("\u{093E}", '\u{0945}') => '\u{0949}', // ा ॅ → ॉ
        ("\u{093E}", '\u{0946}') => '\u{094A}', // ा ॆ → ॊ
        ("\u{093E}", '\u{093A}') => '\u{093B}', // ा ऺ → ऻ
        ("\u{0B47}", '\u{0B3E}') => '\u{0B4B}',
        ("\u{0B47}", '\u{0B57}') => '\u{0B4C}',
        ("\u{0BC6}", '\u{0BBE}') => '\u{0BCA}',
        ("\u{0BC7}", '\u{0BBE}') => '\u{0BCB}',
        ("\u{0BC6}", '\u{0BD7}') => '\u{0BCC}',
        ("\u{0D46}", '\u{0D3E}') => '\u{0D4A}',
        ("\u{0D47}", '\u{0D3E}') => '\u{0D4B}',
        ("\u{0D46}", '\u{0D57}') => '\u{0D4C}',
        _ => return None,
    };
    let mut out = joined.to_string();
    out.extend(rest);
    Some(out)
}

/// Puts one line's pieces from drawing order into reading order:
/// pre-base vowel signs move after their consonant cluster, a reph moves to
/// the start of its syllable, and two-part vowels are joined.
pub(crate) fn reorder(pieces: Vec<GlyphPiece<'_>>) -> Vec<GlyphPiece<'_>> {
    // A reph from the PDF's own text layer has no role yet: "র্" that is
    // not followed by a consonant was drawn after its syllable.
    let mut pieces = pieces;
    for i in 0..pieces.len() {
        if pieces[i].role == Role::Base
            && is_ra_virama(&pieces[i].text)
            && i > 0
            && !pieces
                .get(i + 1)
                .is_some_and(|n| starts_with_letter(&n.text))
        {
            pieces[i].role = Role::Reph;
        }
    }

    // 1. Pre-base vowel signs.
    let mut out: Vec<GlyphPiece<'_>> = Vec::with_capacity(pieces.len());
    let mut pending: Vec<GlyphPiece<'_>> = Vec::new();
    let mut it = pieces.into_iter().peekable();
    while let Some(p) = it.next() {
        if matches!(p.role, Role::PreBase | Role::PreBaseReph) {
            pending.push(p);
            continue;
        }
        let letter = starts_with_letter(&p.text);
        let cluster_start = out.len();
        out.push(p);
        if !pending.is_empty() && letter {
            while let Some(next) = it.peek() {
                let joined = out.last().is_some_and(|q| ends_with_virama(&q.text));
                let prebase = matches!(next.role, Role::PreBase | Role::PreBaseReph);
                if !prebase && (joined || continues_cluster(&next.text)) {
                    out.push(it.next().expect("peeked"));
                } else {
                    break;
                }
            }
            // A vowel sign fused with a reph (Devanagari "िर्" over र्ति):
            // the reph starts the cluster, the vowel sign follows it.
            for mut q in pending.drain(..) {
                if q.role == Role::PreBaseReph
                    && let Some((reph, rest)) = reph_split(&q.text)
                {
                    let r = GlyphPiece {
                        text: reph,
                        role: Role::Base,
                        ..GlyphPiece::new(q.glyph, String::new(), Role::Base)
                    };
                    out.insert(cluster_start, GlyphPiece { rect: q.rect, ..r });
                    q.text = rest;
                }
                q.role = Role::Base;
                out.push(q);
            }
        }
    }
    out.append(&mut pending);

    // 2. Marks drawn before the rest of their cluster: a vowel sign or
    //    candrabindu drawn before a ya-phala (বিদ্যুৎ, ব্যাঁক), candrabindu
    //    drawn before the vowel sign it follows.
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..out.len().saturating_sub(1) {
            let (a, b) = (&out[i].text, &out[i + 1].text);
            let first = |t: &str| t.chars().next().and_then(indic_cat).map(|(_, k)| k);
            let swap = match (first(a), first(b)) {
                (Some(IndicCat::Matra | IndicCat::Sign), _) if continues_cluster(b) => true,
                (Some(IndicCat::Sign), Some(IndicCat::Matra)) => true,
                _ => false,
            };
            if swap {
                out.swap(i, i + 1);
                changed = true;
            }
        }
    }

    // 3. Reph to the start of its syllable. A reph fused with the vowel
    //    sign under it ("ीर्") leaves the vowel sign where it was drawn.
    let mut i = 0;
    while i < out.len() {
        if out[i].role == Role::Reph {
            out[i].role = Role::Base;
            let mut j = i;
            while j > 0 {
                j -= 1;
                let start = starts_with_letter(&out[j].text)
                    && !(j > 0 && ends_with_virama(&out[j - 1].text));
                if start {
                    break;
                }
            }
            if j < i {
                let (reph_text, rest) = reph_split(&out[i].text)
                    .unwrap_or_else(|| (std::mem::take(&mut out[i].text), String::new()));
                out[i].text = rest;
                if out[i].text.is_empty() {
                    let mut reph = out.remove(i);
                    reph.text = reph_text;
                    out.insert(j, reph);
                } else {
                    let reph = GlyphPiece::new(out[i].glyph, reph_text, Role::Base);
                    out.insert(
                        j,
                        GlyphPiece {
                            rect: out[i].rect,
                            ..reph
                        },
                    );
                    i += 1;
                }
            }
        }
        i += 1;
    }

    // 4. Two-part vowels.
    let mut merged: Vec<GlyphPiece<'_>> = Vec::with_capacity(out.len());
    for p in out {
        if let Some(prev) = merged.last_mut()
            && let Some(joined) = compose_split_vowel(&prev.text, &p.text)
        {
            prev.text = joined;
            prev.rect = prev.rect.union(p.rect);
            continue;
        }
        merged.push(p);
    }
    merged
}
