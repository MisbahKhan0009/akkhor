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

/// The pieces of one word (glyphs between visible gaps, in drawing order).
pub(crate) fn word_pieces<'g>(word: &[&'g PdfGlyph], fonts: &[FontReading]) -> Vec<GlyphPiece<'g>> {
    let none = FontReading::default();
    let ids: Vec<u32> = word.iter().map(|g| g.glyph_id).collect();
    let mut out = Vec::with_capacity(word.len());
    let mut i = 0;
    while i < word.len() {
        let g = word[i];
        let font = fonts.get(g.font).unwrap_or(&none);
        let rec = font.table.as_deref();
        let same_font = word[i..].iter().take_while(|x| x.font == g.font).count();

        if font.bijoy
            && let Some(p) = bijoy_piece(&word[i..i + same_font], font)
        {
            out.push(p);
            i += same_font;
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

/// A glyph its font draws as nothing, with width: a space. (A glyph that
/// is merely unknown is not: it may be a letter.)
fn is_space(g: &PdfGlyph, rec: Option<&Recovery>) -> bool {
    rec.is_some_and(|r| r.blank.contains(&g.glyph_id)) && g.advance > g.size * 0.15
}

/// A run of glyphs in a Bijoy font, converted as one piece. `None` when the
/// run already reads as Unicode Bangla (a mislabelled font).
fn bijoy_piece<'g>(run: &[&'g PdfGlyph], font: &FontReading) -> Option<GlyphPiece<'g>> {
    let rec = font.table.as_deref();
    let mut letters: Vec<char> = Vec::new();
    for x in run {
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
