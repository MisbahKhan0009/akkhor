//! OpenType shaping of a paragraph.
//!
//! The paragraph is split into script runs and each run is shaped once with
//! HarfRust, which implements HarfBuzz's Indic shapers: conjunct forms, reph,
//! pre-base matras and below-base forms come out as the font designer drew
//! them. Line breaking then works on the shaped glyphs, so nothing is shaped
//! twice.

use harfrust::{Buffer, Direction, ShapeOptions, ShaperFont, shape};

use crate::fonts::{FontId, FontSet};
use crate::script::{Script, script_runs};

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

/// Shapes `text` at `size` pixels. Glyphs come back in visual order with
/// non-decreasing clusters (left-to-right scripts only, for now).
pub fn shape_paragraph(text: &str, size: f32, fonts: &FontSet) -> Vec<ShapedGlyph> {
    let mut out = Vec::with_capacity(text.len());
    let mut buffer = Buffer::new();
    for (range, script) in script_runs(text) {
        let Some((font_id, font)) = fonts.for_script(script) else {
            continue;
        };
        let scale = size / font.units_per_em;
        buffer.clear();
        buffer.push_str(&text[range.clone()]);
        buffer.set_direction(Direction::LeftToRight);
        buffer.set_script(hr_script(script));
        buffer.guess_segment_properties();
        let shaper = ShaperFont::new(&font.font);
        if shape(&shaper, &mut buffer, ShapeOptions::new()).is_err() {
            continue;
        }
        for (info, pos) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
            out.push(ShapedGlyph {
                font: font_id,
                glyph: info.glyph_id,
                cluster: range.start + info.cluster as usize,
                advance: pos.x_advance as f32 * scale,
                x_offset: pos.x_offset as f32 * scale,
                y_offset: pos.y_offset as f32 * scale,
                size,
            });
        }
    }
    out
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;

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
}
