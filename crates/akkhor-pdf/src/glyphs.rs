//! Reading the glyphs of a page: what was drawn, where, with which font,
//! and what the PDF's own text layer (`ToUnicode`) says it means.
//!
//! This is the raw material for the text layer. Positions are in page
//! points with the origin at the top left and y growing downwards, the same
//! space [`PdfDocument::render_tile`](crate::PdfDocument::render_tile) uses
//! at scale 1.

use std::sync::Arc;

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath, Point, Rect};

/// A font that drew glyphs on the page.
pub struct PdfFont {
    /// Identifies the font program; equal keys mean the same font.
    pub key: u128,
    /// PostScript name with any subset prefix removed (`ABCDEF+SutonnyMJ`
    /// becomes `SutonnyMJ`).
    pub name: Option<String>,
    /// The embedded font program (TrueType, OpenType or CFF), when there is one.
    pub data: Option<Arc<dyn AsRef<[u8]> + Send + Sync>>,
}

impl std::fmt::Debug for PdfFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfFont")
            .field("name", &self.name)
            .field("embedded", &self.data.is_some())
            .finish()
    }
}

/// One drawn glyph.
#[derive(Clone, Debug)]
pub struct PdfGlyph {
    /// Text from the PDF's `ToUnicode` map or the glyph's name, if any.
    pub unicode: Option<String>,
    pub char_code: u32,
    /// Glyph id in the font program (0 for Type 3 glyphs).
    pub glyph_id: u32,
    /// Index into [`PageGlyphs::fonts`].
    pub font: usize,
    /// Baseline origin.
    pub origin: (f32, f32),
    /// Advance along the baseline, in points.
    pub advance: f32,
    /// Font size in points.
    pub size: f32,
    /// Box from ascender to descender over the advance, in points.
    pub rect: Rect,
}

/// The glyphs of one page and the fonts they use.
#[derive(Debug, Default)]
pub struct PageGlyphs {
    pub glyphs: Vec<PdfGlyph>,
    pub fonts: Vec<Arc<PdfFont>>,
}

fn strip_subset_prefix(name: &str) -> String {
    match name.split_once('+') {
        Some((prefix, rest))
            if prefix.len() == 6 && prefix.bytes().all(|b| b.is_ascii_uppercase()) =>
        {
            rest.to_string()
        }
        _ => name.to_string(),
    }
}

struct Collector {
    out: PageGlyphs,
}

impl Collector {
    fn font_index(&mut self, glyph: &Glyph<'_>) -> usize {
        let (key, name, data) = match glyph {
            Glyph::Outline(o) => match o.font_data() {
                Some(d) => (
                    d.cache_key,
                    d.postscript_name.as_deref().map(strip_subset_prefix),
                    Some(d.data),
                ),
                None => (0, None, None),
            },
            Glyph::Type3(_) => (1, None, None),
        };
        if let Some(i) = self.out.fonts.iter().position(|f| f.key == key) {
            return i;
        }
        self.out.fonts.push(Arc::new(PdfFont { key, name, data }));
        self.out.fonts.len() - 1
    }
}

impl<'a> Device<'a> for Collector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: hayro::hayro_interpret::Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let Some(first) = run.glyphs().first() else {
            return;
        };
        // All glyphs of a run share one font.
        let font = self.font_index(first);
        for g in run.glyphs() {
            // Glyph space is 1000 units per em.
            let t: Affine = props.transform * g.transform();
            let origin = t * Point::ZERO;
            let up = t * Point::new(0.0, 1000.0) - origin;
            let size = up.hypot() as f32;
            let (advance_units, glyph_id) = match &**g {
                Glyph::Outline(o) => (o.advance_width().unwrap_or(500.0), o.glyph_id().to_u32()),
                Glyph::Type3(t3) => (t3.advance_width().unwrap_or(500.0) * 1000.0, 0),
            };
            let end = t * Point::new(advance_units as f64, 0.0);
            let advance = (end - origin).hypot() as f32;
            let top = origin.y - 0.8 * size as f64;
            let bottom = origin.y + 0.25 * size as f64;
            let unicode = g.as_unicode().map(|b| match b {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            });
            self.out.glyphs.push(PdfGlyph {
                unicode,
                char_code: g.char_code(),
                glyph_id,
                font,
                origin: (origin.x as f32, origin.y as f32),
                advance,
                size,
                rect: Rect::new(origin.x.min(end.x), top, origin.x.max(end.x), bottom),
            });
        }
    }
}

/// Collects the glyphs drawn on `page`.
pub(crate) fn collect(page: &Page<'_>) -> PageGlyphs {
    let (w, h) = page.render_dimensions();
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        page.initial_transform(true).to_kurbo_affine(),
        Rect::new(0.0, 0.0, w as f64, h as f64),
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut collector = Collector {
        out: PageGlyphs::default(),
    };
    interpret_page(page, &mut ctx, &mut collector);
    collector.out
}

trait ToKurbo {
    fn to_kurbo_affine(self) -> Affine;
}

impl<T: hayro::hayro_interpret::util::TransformExt> ToKurbo for T {
    fn to_kurbo_affine(self) -> Affine {
        self.to_kurbo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset_prefixes_are_removed() {
        assert_eq!(strip_subset_prefix("ABCDEF+SutonnyMJ"), "SutonnyMJ");
        assert_eq!(strip_subset_prefix("Kruti+Dev"), "Kruti+Dev");
        assert_eq!(strip_subset_prefix("NotoSerif"), "NotoSerif");
    }
}
