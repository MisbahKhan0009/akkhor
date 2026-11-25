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
    /// What each line looks like, for page structure (headings, columns,
    /// running headers); parallel to `lines`.
    pub line_info: Vec<LineInfo>,
}

/// The look and place of one line of a page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineInfo {
    /// Box of the line's glyphs, in points (top-left origin).
    pub rect: Rect,
    /// Baseline, in points from the top.
    pub baseline: f32,
    /// Font size in points (the most common on the line).
    pub size: f32,
    /// Most glyphs bold: a bold font, or text stroked or drawn twice.
    pub bold: bool,
    /// Most glyphs in an italic or oblique font.
    pub italic: bool,
    /// Every glyph tagged by the producer as an artifact (running header,
    /// page number).
    pub artifact: bool,
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
    ///
    /// A word split by a line-end hyphen (শি-⏎ক্ষার্থী) is found too: the
    /// text is also searched with such hyphens joined, and the match covers
    /// both halves.
    pub fn find(&self, query: &str) -> Vec<Range<usize>> {
        let q = akkhor_flow::search::fold(query);
        let mut out: Vec<Range<usize>> = akkhor_flow::search::find_in_text(&self.text, &q)
            .into_iter()
            .map(|(s, e)| s..e)
            .collect();
        // Joined both ways: শি-⏎ক্ষা is শিক্ষা, and ধীরে-⏎ধীরে is ধীরে-ধীরে.
        for keep in [false, true] {
            let (joined, origin) = joined_hyphens(&self.text, keep);
            if joined.len() == self.text.len() {
                break;
            }
            for (s, e) in akkhor_flow::search::find_in_text(&joined, &q) {
                let range = origin[s]..origin[e];
                // Only matches that run across a joined line end are new.
                if self.text[range.clone()].contains('\n')
                    && !out
                        .iter()
                        .any(|r| r.start < range.end && range.start < r.end)
                {
                    out.push(range);
                }
            }
        }
        out.sort_by_key(|r| r.start);
        out
    }
}

fn is_line_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{AD}')
}

/// The text with every `hyphen ⏎ letter` line end joined (the hyphen kept
/// or dropped), and for each byte of it (and its end) the offset in the
/// original text.
fn joined_hyphens(text: &str, keep_hyphen: bool) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len() + 1);
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if is_line_hyphen(c) {
            let mut look = chars.clone();
            if let Some((_, '\n')) = look.next()
                && let Some(&(_, next)) = look.peek()
                && (next.is_alphabetic() || akkhor_flow::script::indic_cat(next).is_some())
                && out.chars().next_back().is_some_and(|p| {
                    p.is_alphabetic() || akkhor_flow::script::indic_cat(p).is_some()
                })
            {
                chars.next(); // the line break
                if keep_hyphen {
                    out.push(c);
                    origin.extend(std::iter::repeat_n(at, c.len_utf8()));
                }
                continue;
            }
        }
        out.push(c);
        origin.extend(std::iter::repeat_n(at, c.len_utf8()));
    }
    origin.push(text.len());
    (out, origin)
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
/// Splits lines where they cross a gutter between columns (or table
/// cells): a gap of over 0.6 em that lines up with such gaps on at least
/// four other lines of the page and that fewer lines run across than leave
/// it open. A loose justified line has wide word gaps too, but other lines
/// of its paragraph run across them. Producers that draw
/// a page row by row put both columns on one baseline.
fn split_at_gutters(lines: Vec<Vec<&PdfGlyph>>) -> Vec<Vec<&PdfGlyph>> {
    // Gaps per line: (line, index of the glyph after the gap, x0, x1, size).
    let mut gaps: Vec<(usize, usize, f32, f32, f32)> = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let mut end = f32::MIN;
        for (i, g) in line.iter().enumerate() {
            let size = g.size.max(1.0);
            if i > 0 && g.origin.0 - end > size * 0.6 {
                gaps.push((li, i, end, g.origin.0, size));
            }
            end = end.max(g.origin.0 + g.advance);
        }
    }
    if gaps.is_empty() {
        return lines;
    }
    let mut cuts: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
    for &(li, i, x0, x1, _) in &gaps {
        let lined_up = gaps
            .iter()
            .filter(|&&(lj, _, y0, y1, _)| lj != li && y0 < x1 && x0 < y1)
            .map(|&(lj, ..)| lj)
            .collect::<std::collections::HashSet<_>>()
            .len();
        // Lines whose glyphs run across the middle of the gap: in a column
        // layout only full-width lines do; in a loose paragraph most do.
        let mid = (x0 + x1) / 2.0;
        let crossing = lines
            .iter()
            .filter(|l| {
                l.iter()
                    .any(|g| g.origin.0 < mid && mid < g.origin.0 + g.advance)
            })
            .count();
        if lined_up >= 4 && crossing < lined_up {
            cuts[li].push(i);
        }
    }
    let mut out = Vec::with_capacity(lines.len());
    for (line, cut) in lines.into_iter().zip(cuts) {
        if cut.is_empty() {
            out.push(line);
            continue;
        }
        let mut start = 0;
        for c in cut {
            out.push(line[start..c].to_vec());
            start = c;
        }
        out.push(line[start..].to_vec());
    }
    out
}

/// The smallest gap, in ems, that counts as a space on this line.
///
/// Producers that draw no space glyphs (XeLaTeX, InDesign) leave gaps that a
/// justified line squeezes to about 0.2 em, while the gaps between the
/// glyphs of a word are zero or kerning-sized. The line's own word gaps
/// set the threshold: 60% of their median, between 0.1 and 0.22 em.
fn word_gap(line: &[&PdfGlyph]) -> f32 {
    const DEFAULT: f32 = 0.22;
    let mut pen_end: Option<f32> = None;
    let mut gaps: Vec<f32> = Vec::new();
    for g in line {
        if let Some(end) = pen_end {
            let gap = (g.origin.0 - end) / g.size.max(1.0);
            if (0.1..1.5).contains(&gap) {
                gaps.push(gap);
            }
        }
        let end = g.origin.0 + g.advance;
        pen_end = Some(pen_end.map_or(end, |e: f32| e.max(end)));
    }
    if gaps.len() < 2 {
        return DEFAULT;
    }
    gaps.sort_by(f32::total_cmp);
    (gaps[gaps.len() / 2] * 0.6).clamp(0.1, DEFAULT)
}

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
    let lines = split_at_gutters(lines);

    let mut out = PageText::default();
    let mut prev_line: Option<(f32, f32, f32)> = None; // baseline, size, left
    for line in lines {
        // Words first, in drawing order: a visible gap between glyphs is a
        // space the PDF did not draw. Reordering then stays inside a word.
        let mut spans: Vec<Range<usize>> = Vec::new();
        let mut pen_end: Option<f32> = None;
        let threshold = word_gap(&line);
        for (i, g) in line.iter().enumerate() {
            // A superscript (a footnote marker) is a word of its own: smaller
            // and raised or lowered against its neighbour.
            let shifted = i > 0 && {
                let p = line[i - 1];
                let (big, small) = if p.size > g.size {
                    (p.size, g.size)
                } else {
                    (g.size, p.size)
                };
                big > small * 1.3 && (p.origin.1 - g.origin.1).abs() > small * 0.15
            };
            let gap = shifted || pen_end.is_some_and(|end| g.origin.0 - end > g.size * threshold);
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
        out.line_info.push(line_info(&line, page));
        prev_line = Some((first.origin.1, first.size, first.origin.0));
    }
    out
}

fn line_info(line: &[&PdfGlyph], page: &PageGlyphs) -> LineInfo {
    let visible: Vec<&&PdfGlyph> = line
        .iter()
        .filter(|g| g.advance > 0.0 || g.size > 0.0)
        .collect();
    let rect = visible
        .iter()
        .map(|g| g.rect)
        .reduce(|a, b| a.union(b))
        .unwrap_or_default();
    let mut sizes: Vec<f32> = visible
        .iter()
        .map(|g| (g.size * 4.0).round() / 4.0)
        .collect();
    sizes.sort_by(f32::total_cmp);
    // The most common size: sizes sorted, the longest run.
    let mut size = sizes.first().copied().unwrap_or(0.0);
    let (mut run, mut best) = (0, 0);
    for (i, s) in sizes.iter().enumerate() {
        run = if i > 0 && sizes[i - 1] == *s {
            run + 1
        } else {
            1
        };
        if run > best {
            best = run;
            size = *s;
        }
    }
    let name = |g: &PdfGlyph| {
        page.fonts
            .get(g.font)
            .and_then(|f| f.name.as_deref())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let half = visible.len().div_ceil(2).max(1);
    let bold = visible
        .iter()
        .filter(|g| {
            let n = name(g);
            g.bold
                || ["bold", "black", "heavy", "semibold", "demi"]
                    .iter()
                    .any(|w| n.contains(w))
        })
        .count()
        >= half;
    let italic = visible
        .iter()
        .filter(|g| {
            let n = name(g);
            n.contains("italic") || n.contains("oblique")
        })
        .count()
        >= half;
    LineInfo {
        rect,
        baseline: line.first().map_or(0.0, |g| g.origin.1),
        size,
        bold,
        italic,
        artifact: !line.is_empty() && line.iter().all(|g| g.artifact),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_words_split_by_a_line_end_hyphen() {
        let t = PageText {
            text: "স্কুলে শি-\nক্ষার্থীরা আসে। পাঠ-ইঞ্জিন".into(),
            ..PageText::default()
        };
        let hits = t.find("শিক্ষার্থীরা");
        assert_eq!(hits.len(), 1);
        assert_eq!(&t.text[hits[0].clone()], "শি-\nক্ষার্থীরা");
        // A compound inside a line is found as written, once.
        assert_eq!(t.find("পাঠ-ইঞ্জিন").len(), 1);
        let t2 = PageText {
            text: "धीरे-\nधीरे चलो".into(),
            ..PageText::default()
        };
        assert_eq!(t2.find("धीरे-धीरे").len(), 1);
        assert_eq!(t2.find("धीरेधीरे").len(), 1);
        // Text with no hyphenated line end is searched once.
        assert_eq!(t.find("আসে").len(), 1);
    }
}
