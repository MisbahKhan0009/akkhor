//! Turning one word's glyphs into text pieces: which source of letters to
//! believe for each font, glyph runs that mean more than their parts, and
//! Bijoy runs.

use std::sync::Arc;

use crate::glyphs::{PageGlyphs, PdfGlyph};
use crate::recover::{self, Recovery, Role};
use crate::text::{self, GlyphPiece};

/// How to read the glyphs of one font on a page.
#[derive(Clone, Default)]
pub(crate) struct FontReading {
    /// Letters recovered from the glyphs, if any.
    pub table: Option<Arc<Recovery>>,
    /// Whether the PDF's own text layer is believed for this font.
    pub trust_text_layer: bool,
    /// Bijoy: Latin-1 letters drawn in a Bangla-looking font.
    pub bijoy: bool,
}

/// Decides, per font, between the text layer and the recovered letters.
///
/// The two are compared glyph by glyph. When they mostly disagree, a
/// complete font (the embedded one with its shaping rules, or a matched
/// reference) wins over the text layer, and the text layer wins over a
/// subset's bare character map, which some producers fill with their own
/// codes. Without a ToUnicode map the interpreter guesses text from
/// character codes, which this also catches.
pub(crate) fn readings(
    glyphs: &PageGlyphs,
    tables: Vec<Option<Arc<Recovery>>>,
) -> Vec<FontReading> {
    let mut votes = vec![(0usize, 0usize); glyphs.fonts.len()];
    for g in &glyphs.glyphs {
        let rec = tables.get(g.font).and_then(|t| t.as_deref());
        if let (Some(own), Some(r)) = (
            g.unicode
                .as_deref()
                .filter(|u| text::usable(u) && !u.trim().is_empty()),
            rec.and_then(|r| r.map.get(&g.glyph_id)),
        ) && let Some(v) = votes.get_mut(g.font)
        {
            v.1 += 1;
            v.0 += usize::from(recover::canonical(own) == r.text);
        }
    }
    tables
        .into_iter()
        .zip(&glyphs.fonts)
        .zip(votes)
        .map(|((mut table, font), (agree, n))| {
            let mut trust_text_layer = true;
            if n > 0 && agree * 2 < n {
                if table.as_ref().is_some_and(|t| t.complete) {
                    trust_text_layer = false;
                } else {
                    table = None;
                }
            }
            FontReading {
                table,
                trust_text_layer,
                bijoy: font.name.as_deref().is_some_and(crate::is_bijoy_font),
            }
        })
        .collect()
}

/// The `/ActualText` spans of a page: their text, and the glyph each one's
/// text is given to (its first, in drawing order).
pub(crate) struct ActualTexts<'g> {
    texts: &'g [String],
    first: Vec<Option<*const PdfGlyph>>,
}

impl<'g> ActualTexts<'g> {
    pub(crate) fn new(glyphs: &'g PageGlyphs) -> Self {
        let mut first = vec![None; glyphs.actual_texts.len()];
        for g in &glyphs.glyphs {
            if let Some(span) = g.actual
                && let Some(slot) = first.get_mut(span)
                && slot.is_none()
            {
                *slot = Some(g as *const PdfGlyph);
            }
        }
        Self {
            texts: &glyphs.actual_texts,
            first,
        }
    }

    /// For a glyph inside a usable ActualText span: `Some(text)` for the
    /// span's first glyph, `Some("")` for the others. `None` outside.
    fn of(&self, g: &PdfGlyph) -> Option<&'g str> {
        let span = g.actual?;
        let text = self.texts.get(span)?;
        if text.trim().is_empty() || !text::usable(text.trim()) {
            return None;
        }
        let first = self.first.get(span).copied().flatten()?;
        Some(if std::ptr::eq(first, g) {
            text.as_str()
        } else {
            ""
        })
    }
}

/// The pieces of one word (glyphs between visible gaps, in drawing order).
pub(crate) fn word_pieces<'g>(
    word: &[&'g PdfGlyph],
    fonts: &[FontReading],
    actual: &ActualTexts<'_>,
) -> Vec<GlyphPiece<'g>> {
    let none = FontReading::default();
    let ids: Vec<u32> = word.iter().map(|g| g.glyph_id).collect();
    let mut out: Vec<GlyphPiece<'g>> = Vec::with_capacity(word.len());
    let mut i = 0;
    // Text of the ActualText span that made the last piece, if it did.
    let mut last_span: Option<String> = None;
    // Glyphs the last span's piece holds: a cluster split into one-glyph
    // spans repeating its text holds at most one glyph per character.
    let mut span_glyphs = 0usize;
    while i < word.len() {
        let g = word[i];
        if g.actual.is_none() {
            // Some producers (Typst) close a span before the cluster's last
            // mark or reph: a glyph drawn right after the span whose text the
            // span already holds belongs to it.
            if let (Some(text), Some(prev)) = (last_span.as_deref(), out.last_mut())
                && continues_span(g, text, prev.rect, fonts, rec_of(fonts, g))
            {
                prev.rect = prev.rect.union(g.rect);
                i += 1;
                continue;
            }
            last_span = None;
        }
        let font = fonts.get(g.font).unwrap_or(&none);
        let rec = font.table.as_deref();
        let same_font = word[i..].iter().take_while(|x| x.font == g.font).count();

        if font.bijoy
            && let Some(p) = bijoy_piece(&word[i..i + same_font], font, actual)
        {
            out.push(p);
            i += same_font;
            continue;
        }

        // The producer's own logical text for this cluster (/ActualText):
        // given to the span's first glyph; the rest of the span in this word
        // joins its box.
        if let Some(t) = actual.of(g) {
            let span = g.actual;
            let n = word[i..].iter().take_while(|x| x.actual == span).count();
            if !t.is_empty() {
                let text = recover::canonical(t.trim_end());
                // ... or splits one cluster into two spans with the same
                // text, the second holding only its marks.
                let room = span_glyphs + n <= text.chars().count();
                if last_span.as_deref() == Some(text.as_str())
                    && let Some(prev) = out.last_mut()
                    && word[i..i + n].iter().all(|x| {
                        sits_over(x, prev.rect)
                            || continues_span(x, &text, prev.rect, fonts, rec_of(fonts, x))
                            || (room && n == 1 && rec_of(fonts, x).is_none())
                    })
                {
                    for x in &word[i..i + n] {
                        prev.rect = prev.rect.union(x.rect);
                    }
                    span_glyphs += n;
                    i += n;
                    continue;
                }
                // ... or draws the cluster's base before the span and puts
                // only its marks inside, with the whole cluster's text.
                // The text spells a reph first (র্ড) though the base was
                // drawn first: the base's letters follow it.
                let after_reph: String = {
                    let mut c = text.chars();
                    match (c.next(), c.next()) {
                        (Some(r), Some(v))
                            if matches!(r, '\u{09B0}' | '\u{0930}')
                                && matches!(v, '\u{09CD}' | '\u{094D}') =>
                        {
                            c.collect()
                        }
                        _ => String::new(),
                    }
                };
                if last_span.is_none()
                    && let Some(prev) = out.last_mut()
                    && !prev.text.is_empty()
                    && (text.starts_with(prev.text.as_str())
                        || after_reph.starts_with(prev.text.as_str()))
                    && word[i..i + n].iter().all(|x| sits_over(x, prev.rect))
                {
                    prev.text = text.clone();
                    for x in &word[i..i + n] {
                        prev.rect = prev.rect.union(x.rect);
                    }
                    last_span = Some(text);
                    span_glyphs = 1 + n;
                    i += n;
                    continue;
                }
                last_span = Some(text.clone());
                span_glyphs = n;
                let mut p = GlyphPiece::new(g, text, Role::Base);
                for x in &word[i + 1..i + n] {
                    p.rect = p.rect.union(x.rect);
                }
                out.push(p);
            }
            i += n;
            continue;
        }

        // A run of glyphs that means more than its parts (ई drawn as इ and
        // the reph glyph), longest first.
        if let Some(r) = rec.filter(|r| r.max_sequence >= 2) {
            let longest = r.max_sequence.min(same_font);
            let hit = (2..=longest)
                .rev()
                .find_map(|n| r.sequences.get(&ids[i..i + n]).map(|t| (n, t)));
            if let Some((n, t)) = hit {
                let mut p = GlyphPiece::new(g, t.clone(), Role::Base);
                for x in &word[i + 1..i + n] {
                    p.rect = p.rect.union(x.rect);
                }
                out.push(p);
                i += n;
                continue;
            }
        }

        // An invisible, zero-width glyph in a recovered font is a shaping
        // leftover, whatever the text layer says.
        if g.advance == 0.0 && rec.is_some_and(|r| r.indic && r.blank.contains(&g.glyph_id)) {
            i += 1;
            continue;
        }

        let recovered = rec.and_then(|r| r.map.get(&g.glyph_id));
        let own = g
            .unicode
            .as_deref()
            .filter(|u| font.trust_text_layer && text::usable(u));
        // Some producers (Typst) map every glyph of a cluster to the whole
        // cluster's text: a mark that repeats the text of the glyph it sits
        // on adds nothing.
        if recovered.is_none()
            && let (Some(t), Some(prev)) = (own, out.last_mut())
            && !t.is_empty()
            && recover::canonical(t) == prev.text
            && sits_over(g, prev.rect)
        {
            prev.rect = prev.rect.union(g.rect);
            i += 1;
            continue;
        }
        let (t, role) = match (recovered, own) {
            (Some(r), _) if rec.is_some_and(|r| r.indic) => (r.text.clone(), r.role),
            (_, Some(t)) => (recover::canonical(t), recover::role_of(t)),
            (Some(r), None) => (r.text.clone(), r.role),
            (None, None) if is_space(g, rec) => (" ".to_string(), Role::Base),
            (None, None) => (String::new(), Role::Base),
        };
        out.push(GlyphPiece::new(g, t, role));
        i += 1;
    }
    out
}

fn rec_of<'a>(fonts: &'a [FontReading], g: &PdfGlyph) -> Option<&'a Recovery> {
    fonts.get(g.font).and_then(|f| f.table.as_deref())
}

/// Marks and the reph sit over their cluster's base: they start inside it,
/// or have no advance of their own and start where it ends.
fn sits_over(x: &PdfGlyph, base: hayro::kurbo::Rect) -> bool {
    let zero_width = x.advance <= x.size * 0.05;
    let slack = f64::from(x.size) * 0.05;
    x.rect.x0 < base.x1 - slack.min(0.1) || (zero_width && x.rect.x0 <= base.x1 + slack)
}

/// Whether `x`, drawn right after an ActualText span reading `span_text`,
/// is a mark of the span's last cluster that the producer left outside it.
fn continues_span(
    x: &PdfGlyph,
    span_text: &str,
    span_rect: hayro::kurbo::Rect,
    fonts: &[FontReading],
    rec: Option<&Recovery>,
) -> bool {
    let over = sits_over(x, span_rect);
    let is_mark = |t: &str| {
        !t.is_empty()
            && t.chars().all(|c| {
                matches!(
                    akkhor_flow::script::indic_cat(c),
                    Some((
                        _,
                        akkhor_flow::script::IndicCat::Matra
                            | akkhor_flow::script::IndicCat::Virama
                            | akkhor_flow::script::IndicCat::Nukta
                            | akkhor_flow::script::IndicCat::Sign
                    ))
                )
            })
    };
    if let Some(r) = rec.and_then(|r| r.map.get(&x.glyph_id)) {
        // A fragment starting with a virama (a ya-phala) cannot start a
        // cluster of its own either.
        let fragment = r.text.chars().next().is_some_and(|c| {
            matches!(
                akkhor_flow::script::indic_cat(c),
                Some((_, akkhor_flow::script::IndicCat::Virama))
            )
        });
        return !r.text.is_empty()
            && span_text.contains(r.text.as_str())
            && (is_mark(&r.text) || fragment || over);
    }
    // No recovered table: the text layer may map the mark to its whole
    // cluster, which then repeats the span's text exactly.
    let trusted = fonts.get(x.font).is_some_and(|f| f.trust_text_layer);
    x.unicode.as_deref().is_some_and(|u| {
        let u = recover::canonical(u);
        let next_to = x.rect.x0 <= span_rect.x1 + f64::from(x.size) * 0.05;
        trusted
            && !u.is_empty()
            && span_text.contains(u.as_str())
            && (over || is_mark(&u) || (u == span_text && next_to))
    })
}

/// A glyph its font draws as nothing, with width: a space. (A glyph that
/// is merely unknown is not: it may be a letter.)
fn is_space(g: &PdfGlyph, rec: Option<&Recovery>) -> bool {
    rec.is_some_and(|r| r.blank.contains(&g.glyph_id)) && g.advance > g.size * 0.15
}

/// A run of glyphs in a Bijoy font, converted as one piece. `None` when the
/// run already reads as Unicode Bangla (a mislabelled font).
fn bijoy_piece<'g>(
    run: &[&'g PdfGlyph],
    font: &FontReading,
    actual: &ActualTexts<'_>,
) -> Option<GlyphPiece<'g>> {
    let rec = font.table.as_deref();
    let mut letters: Vec<char> = Vec::new();
    for x in run {
        if let Some(t) = actual.of(x) {
            letters.extend(t.chars());
            continue;
        }
        let own = x
            .unicode
            .as_deref()
            .filter(|u| font.trust_text_layer && text::usable(u));
        let recovered = rec
            .and_then(|r| r.map.get(&x.glyph_id))
            .map(|t| t.text.as_str());
        match own.or(recovered) {
            Some(t) => letters.extend(t.chars()),
            None if is_space(x, rec) => letters.push(' '),
            None => {}
        }
    }
    if letters
        .iter()
        .any(|c| ('\u{0980}'..='\u{09FF}').contains(c))
    {
        return None;
    }
    // ‚ (ূ-kar) and the comma share one outline in many fonts; inside a
    // word it is never a comma.
    for k in 0..letters.len().saturating_sub(1) {
        if letters[k] == ',' && letters[k + 1].is_alphanumeric() {
            letters[k] = '\u{201A}';
        }
    }
    let ascii: String = letters.into_iter().collect();
    let mut p = GlyphPiece::new(
        run[0],
        akkhor_flow::bijoy::bijoy_to_unicode(&ascii),
        Role::Base,
    );
    for x in &run[1..] {
        p.rect = p.rect.union(x.rect);
    }
    Some(p)
}
