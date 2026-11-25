//! OpenType shaping of a paragraph.
//!
//! The paragraph is split into runs of one script, one style and one font,
//! and each run is shaped once with HarfRust, which implements HarfBuzz's
//! Indic shapers: conjunct forms, reph, pre-base matras and below-base forms
//! come out as the font designer drew them. Line breaking then works on the
//! shaped glyphs, so nothing is shaped twice.
//!
//! A character the script's font does not have (a currency sign in a Bangla
//! sentence, say) falls back to the next font that has it. Combining marks
//! and joiners always stay with the font of the letter they attach to, so
//! fallback never splits a syllable across fonts.

use std::ops::Range;

use harfrust::{Buffer, Direction, ShapeOptions, ShaperFont, shape};

use crate::fonts::{FontId, FontSet, FontStyle, Synthetic};
use crate::model::StyleSpan;
use crate::script::{IndicCat, Script, indic_cat, script_runs};

/// One shaped glyph, in pixels at the requested size.
#[derive(Clone, Copy, Debug)]
pub struct ShapedGlyph {
    pub font: FontId,
    pub glyph: u32,
    /// Byte offset into the paragraph text of the cluster this glyph is part of.
    pub cluster: usize,
    pub advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    /// Font size in pixels this glyph was shaped at.
    pub size: f32,
    pub synthetic: Synthetic,
}

fn hr_script(s: Script) -> Option<harfrust::Script> {
    Some(match s {
        Script::Latin => harfrust::Script::LATIN,
        Script::Bengali => harfrust::Script::BENGALI,
        Script::Devanagari => harfrust::Script::DEVANAGARI,
        Script::Gurmukhi => harfrust::Script::GURMUKHI,
        Script::Gujarati => harfrust::Script::GUJARATI,
        Script::Oriya => harfrust::Script::ORIYA,
        Script::Tamil => harfrust::Script::TAMIL,
        Script::Telugu => harfrust::Script::TELUGU,
        Script::Kannada => harfrust::Script::KANNADA,
        Script::Malayalam => harfrust::Script::MALAYALAM,
        Script::Common | Script::Other => return None,
    })
}

/// Characters that must stay in the font of the character before them.
fn attaches_to_previous(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}')
        || matches!(c as u32, 0x0300..=0x036F)
        || matches!(
            indic_cat(c),
            Some((
                _,
                IndicCat::Matra | IndicCat::Virama | IndicCat::Nukta | IndicCat::Sign
            ))
        )
}

/// A run shaped with one font.
struct Run {
    range: Range<usize>,
    script: Script,
    font: FontId,
    synthetic: Synthetic,
}

fn style_at(spans: &[StyleSpan], base: FontStyle, offset: usize) -> FontStyle {
    let mut style = base;
    for s in spans {
        if s.start <= offset && offset < s.end {
            style.bold |= s.style.bold;
            style.italic |= s.style.italic;
        }
    }
    style
}

/// Splits `text` into runs of one script, style and font.
fn itemize(text: &str, spans: &[StyleSpan], base: FontStyle, fonts: &FontSet) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (range, script) in script_runs(text) {
        let mut current: Option<(FontId, Synthetic, FontStyle)> = None;
        for (i, c) in text[range.clone()].char_indices() {
            let at = range.start + i;
            let end = at + c.len_utf8();
            let style = style_at(spans, base, at);
            let pick = match current {
                Some((font, synth, cur_style))
                    if cur_style == style
                        && (attaches_to_previous(c)
                            || fonts.get(font).is_some_and(|f| f.has_char(c))) =>
                {
                    Some((font, synth))
                }
                _ => fonts.resolve(script, style, c),
            };
            let Some((font, synthetic)) = pick else {
                continue;
            };
            current = Some((font, synthetic, style));
            match runs.last_mut() {
                Some(r)
                    if r.range.end == at
                        && r.font == font
                        && r.synthetic == synthetic
                        && r.script == script =>
                {
                    r.range.end = end
                }
                _ => runs.push(Run {
                    range: at..end,
                    script,
                    font,
                    synthetic,
                }),
            }
        }
    }
    runs
}

/// Shapes `text` at `size` pixels in the regular style.
pub fn shape_paragraph(text: &str, size: f32, fonts: &FontSet) -> Vec<ShapedGlyph> {
    shape_styled(text, &[], FontStyle::REGULAR, size, fonts)
}

/// Shapes `text` with bold/italic `spans` (byte ranges into `text`) on top
/// of a `base` style. Glyphs come back in visual order with non-decreasing
/// clusters (left-to-right scripts only, for now).
pub fn shape_styled(
    text: &str,
    spans: &[StyleSpan],
    base: FontStyle,
    size: f32,
    fonts: &FontSet,
) -> Vec<ShapedGlyph> {
    let mut out = Vec::with_capacity(text.len());
    let mut buffer = Buffer::new();
    for run in itemize(text, spans, base, fonts) {
        let Some(font) = fonts.get(run.font) else {
            continue;
        };
        let scale = size / font.units_per_em;
        buffer.clear();
        buffer.push_str(&text[run.range.clone()]);
        buffer.set_direction(Direction::LeftToRight);
        buffer.set_script(hr_script(run.script));
        buffer.guess_segment_properties();
        let shaper = ShaperFont::new(&font.font);
        if shape(&shaper, &mut buffer, ShapeOptions::new()).is_err() {
            continue;
        }
        // Synthetic bold makes strokes thicker; give letters a little room.
        let extra = if run.synthetic.bold { size * 0.02 } else { 0.0 };
        for (info, pos) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
            out.push(ShapedGlyph {
                font: run.font,
                glyph: info.glyph_id,
                cluster: run.range.start + info.cluster as usize,
                advance: pos.x_advance as f32 * scale
                    + if pos.x_advance != 0 { extra } else { 0.0 },
                x_offset: pos.x_offset as f32 * scale,
                y_offset: pos.y_offset as f32 * scale,
                size,
                synthetic: run.synthetic,
            });
        }
    }
    out
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;
    use crate::fonts::FontData;
    use crate::model::Style;

    #[test]
    fn bangla_conjunct_becomes_one_glyph() {
        let fonts = FontSet::bundled();
        // ক্ষ is three code points; Noto Serif Bengali draws it as one ligature.
        let g = shape_paragraph("ক্ষ", 20.0, &fonts);
        assert_eq!(g.len(), 1, "{g:?}");
        // The pre-base vowel sign of কি is drawn before the consonant.
        let g = shape_paragraph("কি", 20.0, &fonts);
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|x| x.cluster == 0));
    }

    #[test]
    fn devanagari_shapes() {
        let fonts = FontSet::bundled();
        let g = shape_paragraph("क्ष", 20.0, &fonts);
        assert_eq!(g.len(), 1, "{g:?}");
        let width: f32 = shape_paragraph("नमस्ते", 20.0, &fonts)
            .iter()
            .map(|g| g.advance)
            .sum();
        assert!(width > 20.0);
    }

    #[test]
    fn missing_characters_fall_back_to_another_font() {
        let fonts = FontSet::bundled();
        let bn = fonts.for_script(Script::Bengali).unwrap().1;
        // Pick a character the Bangla font lacks but Noto Serif has.
        let c = ['€', '©', '™', '½', 'ß']
            .into_iter()
            .find(|&c| !bn.has_char(c) && fonts.get(0).unwrap().has_char(c))
            .expect("a Latin-only symbol");
        let text = format!("দাম {c}৫০");
        let g = shape_paragraph(&text, 20.0, &fonts);
        let at = text.find(c).unwrap();
        let sym = g.iter().find(|g| g.cluster == at).unwrap();
        assert_eq!(sym.font, 0, "symbol should come from the Latin font");
        let ba = g.iter().find(|g| g.cluster == 0).unwrap();
        assert_eq!(ba.font, 1, "Bangla letters stay in the Bangla font");
    }

    #[test]
    fn bold_spans_use_bold_faces_or_synthesis() {
        let text = "আমি bold";
        let spans = [StyleSpan {
            start: 0,
            end: text.len(),
            style: Style {
                bold: true,
                italic: false,
            },
        }];
        // No bold face: synthetic bold.
        let fonts = FontSet::bundled();
        let g = shape_styled(text, &spans, FontStyle::REGULAR, 20.0, &fonts);
        assert!(g.iter().all(|g| g.synthetic.bold));
        // With a registered bold face for Latin, Latin text uses it for real.
        let mut fonts = FontSet::bundled();
        let bold = fonts.add_styled(
            &[Script::Latin, Script::Common],
            FontStyle::BOLD,
            FontData::Static(include_bytes!("../../../fonts/NotoSerif-Regular.ttf")),
        );
        let g = shape_styled(text, &spans, FontStyle::REGULAR, 20.0, &fonts);
        let b = g
            .iter()
            .find(|g| g.cluster == text.find('b').unwrap())
            .unwrap();
        assert_eq!(b.font, bold);
        assert!(!b.synthetic.bold);
        let a = g.iter().find(|g| g.cluster == 0).unwrap();
        assert!(a.synthetic.bold, "Bangla has no bold face here");
    }
}
