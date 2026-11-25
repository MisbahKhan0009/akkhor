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
    /// The `/ActualText` span this glyph belongs to: an index into
    /// [`PageGlyphs::actual_texts`]. Producers write the logical text of a
    /// cluster there (Chrome and LibreOffice do for every Indic cluster).
    pub actual: Option<usize>,
    /// Inside an `/Artifact` marked-content section: running headers, page
    /// numbers and other furniture the producer tagged as not content.
    pub artifact: bool,
    /// Drawn with a stroke, or drawn again over itself: synthetic bold.
    pub bold: bool,
}

/// The glyphs of one page and the fonts they use.
#[derive(Debug, Default)]
pub struct PageGlyphs {
    pub glyphs: Vec<PdfGlyph>,
    pub fonts: Vec<Arc<PdfFont>>,
    /// Text of each `/ActualText` span on the page.
    pub actual_texts: Vec<String>,
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
    /// Open marked-content sections: the `/ActualText` span each one
    /// starts, if any.
    marked: Vec<Option<usize>>,
    /// Open marked-content sections tagged `/Artifact`.
    artifacts: Vec<bool>,
    /// Where each glyph was drawn: (font, glyph id, char code, x and y in
    /// half points) to its index, to drop copies drawn on top.
    placed: std::collections::HashMap<(usize, u32, u32, i64, i64), usize>,
}

/// Decodes a PDF text string: UTF-16 with a byte-order mark, else
/// PDFDocEncoding (Latin-1 for the characters that matter here).
pub(crate) fn text_string(bytes: &[u8]) -> String {
    match bytes {
        [0xFE, 0xFF, rest @ ..] => char::decode_utf16(
            rest.as_chunks::<2>()
                .0
                .iter()
                .map(|&p| u16::from_be_bytes(p)),
        )
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect(),
        [0xFF, 0xFE, rest @ ..] => char::decode_utf16(
            rest.as_chunks::<2>()
                .0
                .iter()
                .map(|&p| u16::from_le_bytes(p)),
        )
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect(),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => bytes.iter().map(|&b| b as char).collect(),
    }
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

    fn begin_marked_content(
        &mut self,
        _tag: &[u8],
        properties: Option<&hayro::hayro_syntax::object::Dict<'_>>,
    ) {
        // The outermost ActualText wins: it replaces everything inside it.
        let inside = self.marked.iter().flatten().next().is_some();
        let span = properties
            .filter(|_| !inside)
            .and_then(|p| p.get::<hayro::hayro_syntax::object::String<'_>>(b"ActualText"))
            .map(|t| {
                self.out.actual_texts.push(text_string(t.as_bytes()));
                self.out.actual_texts.len() - 1
            });
        self.marked.push(span);
        self.artifacts.push(_tag == b"Artifact");
    }

    fn end_marked_content(&mut self) {
        self.marked.pop();
        self.artifacts.pop();
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        let stroked = matches!(mode, DrawMode::Stroke(_) | DrawMode::FillAndStroke(..));
        let artifact = self.artifacts.iter().any(|a| *a);
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
            // The same glyph drawn again on itself (fill, then stroke as a
            // second pass: how many tools fake bold) is one glyph, in bold.
            // Positions are bucketed by half a point; a copy within 0.03 em
            // in this bucket or a neighbouring one is the same glyph.
            let at = (origin.x as f32, origin.y as f32);
            let near = size * 0.03;
            let cell = ((at.0 * 2.0).round() as i64, (at.1 * 2.0).round() as i64);
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    let key = (font, glyph_id, g.char_code(), cell.0 + dx, cell.1 + dy);
                    if let Some(&i) = self.placed.get(&key) {
                        let p = &self.out.glyphs[i];
                        if (p.origin.0 - at.0).abs() <= near && (p.origin.1 - at.1).abs() <= near {
                            found = Some(i);
                            break 'search;
                        }
                    }
                }
            }
            if let Some(i) = found {
                self.out.glyphs[i].bold = true;
                continue;
            }
            self.placed.insert(
                (font, glyph_id, g.char_code(), cell.0, cell.1),
                self.out.glyphs.len(),
            );
            self.out.glyphs.push(PdfGlyph {
                unicode,
                char_code: g.char_code(),
                glyph_id,
                font,
                origin: (origin.x as f32, origin.y as f32),
                advance,
                size,
                rect: Rect::new(origin.x.min(end.x), top, origin.x.max(end.x), bottom),
                actual: self.marked.iter().flatten().next().copied(),
                artifact,
                bold: stroked,
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
        marked: Vec::new(),
        artifacts: Vec::new(),
        placed: Default::default(),
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
    fn text_strings_decode() {
        assert_eq!(text_string(&[0xFE, 0xFF, 0x09, 0x95, 0x09, 0xBE]), "কা");
        assert_eq!(text_string(b"Avwg"), "Avwg");
    }

    #[test]
    fn actual_text_spans_are_collected() {
        let path = format!(
            "{}/../../tests/corpus/pdf/bn.libreoffice.pdf",
            env!("CARGO_MANIFEST_DIR")
        );
        let pdf = crate::PdfDocument::open(&akkhor_core::Source::open(path).unwrap()).unwrap();
        let g = pdf.glyphs(0).unwrap();
        assert!(
            g.actual_texts.iter().any(|t| t == "বাং"),
            "{:?}",
            &g.actual_texts[..3]
        );
        assert!(g.glyphs.iter().filter(|x| x.actual.is_some()).count() > g.glyphs.len() / 2);
    }

    #[test]
    fn subset_prefixes_are_removed() {
        assert_eq!(strip_subset_prefix("ABCDEF+SutonnyMJ"), "SutonnyMJ");
        assert_eq!(strip_subset_prefix("Kruti+Dev"), "Kruti+Dev");
        assert_eq!(strip_subset_prefix("NotoSerif"), "NotoSerif");
    }
}
