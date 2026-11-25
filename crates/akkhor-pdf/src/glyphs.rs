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
    SoftMask, interpret_page,
};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape};

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
    /// For a Type 3 glyph without text: index into
    /// [`PageGlyphs::type3_shapes`].
    pub type3: Option<u32>,
    /// Drawn from a Type 3 font (with or without text).
    pub drawn_type3: bool,
}

/// The glyphs of one page and the fonts they use.
#[derive(Debug, Default)]
pub struct PageGlyphs {
    pub glyphs: Vec<PdfGlyph>,
    pub fonts: Vec<Arc<PdfFont>>,
    /// Text of each `/ActualText` span on the page.
    pub actual_texts: Vec<String>,
    /// Shapes of the page's Type 3 glyphs that carry no text, with their
    /// advance (1000-unit ems), for recognition against reference fonts.
    pub type3_shapes: Vec<(crate::shape::Shape, f32)>,
    /// A few shapes of Type 3 glyphs that do carry text, to tell whether
    /// the page's Type 3 fonts are synthetic bold.
    pub type3_probes: Vec<(crate::shape::Shape, f32)>,
    /// Thin vertical lines drawn on the page (table cell borders).
    pub vertical_rules: Vec<Rect>,
}

/// Type 3 glyphs with text pictured per page to tell synthetic bold.
const TYPE3_PROBES: usize = 8;

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
    /// Bounding boxes of the open clip paths, each within the one before:
    /// a glyph drawn outside is not on the page (Chrome draws the start of
    /// the next page's lines and clips them away).
    clips: Vec<Rect>,
    /// Type 3 glyphs already pictured: glyph cache key to shape index.
    type3_seen: std::collections::HashMap<u128, u32>,
    type3_probed: std::collections::HashSet<u128>,
    /// While drawing a Type 3 glyph's procedure for its shape: the paths.
    capture: Option<BezPath>,
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
    fn new() -> Self {
        Collector {
            out: PageGlyphs::default(),
            marked: Vec::new(),
            artifacts: Vec::new(),
            placed: Default::default(),
            clips: Vec::new(),
            type3_seen: Default::default(),
            type3_probed: Default::default(),
            capture: None,
        }
    }

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
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        // Capturing a Type 3 glyph: the fill is the glyph; a stroke on top
        // only makes it bolder.
        if let Some(out) = self.capture.as_mut() {
            if matches!(mode, DrawMode::Fill(_) | DrawMode::FillAndStroke(..)) {
                out.extend(props.transform * path.clone());
            }
            return;
        }
        // Vertical rules: table cell borders, which split a line of text
        // into cells even where the gap is narrow or the table short.
        if !crate::theme::shows_on_white(&props.paint) {
            return;
        }
        let path = props.transform * path.clone();
        match mode {
            DrawMode::Fill(_) => {
                let b = path.bounding_box();
                if b.width() <= 2.5 && b.height() >= 4.0 {
                    self.out.vertical_rules.push(b);
                }
            }
            DrawMode::Stroke(_) | DrawMode::FillAndStroke(..) => {
                for seg in path.segments() {
                    if let hayro::kurbo::PathSeg::Line(l) = seg
                        && (l.p0.x - l.p1.x).abs() < 0.5
                        && (l.p0.y - l.p1.y).abs() >= 4.0
                    {
                        let (x, y0, y1) = (l.p0.x, l.p0.y.min(l.p1.y), l.p0.y.max(l.p1.y));
                        self.out
                            .vertical_rules
                            .push(Rect::new(x - 0.25, y0, x + 0.25, y1));
                    }
                }
            }
            DrawMode::Invisible => {}
        }
    }
    fn push_clip_path(&mut self, clip: &ClipPath) {
        let b = clip.path.bounding_box();
        let next = match self.clips.last() {
            Some(c) => c.intersect(b),
            None => b,
        };
        self.clips.push(next);
    }
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: hayro::hayro_interpret::Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
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
                // Already in units of 1000 per em (hayro-font scales by the
                // font matrix).
                Glyph::Type3(t3) => (t3.advance_width().unwrap_or(500.0), 0),
            };
            let end = t * Point::new(advance_units as f64, 0.0);
            let advance = (end - origin).hypot() as f32;
            let top = origin.y - 0.8 * size as f64;
            let bottom = origin.y + 0.25 * size as f64;
            let unicode = g.as_unicode().map(|b| match b {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            });
            // A Type 3 glyph with no text is only known by its look.
            let type3 = match &**g {
                Glyph::Type3(t3) if unicode.as_deref().is_none_or(|u| !crate::text::usable(u)) => {
                    let key = hayro::hayro_interpret::CacheKey::cache_key(&**t3)
                        ^ u128::from(g.char_code()).rotate_left(64);
                    match self.type3_seen.get(&key) {
                        Some(&i) => Some(i),
                        None => {
                            let shape = type3_shape(t3, &props.paint);
                            let i = self.out.type3_shapes.len() as u32;
                            self.out.type3_shapes.push((shape, advance_units));
                            self.type3_seen.insert(key, i);
                            Some(i)
                        }
                    }
                }
                Glyph::Type3(t3) => {
                    let key = hayro::hayro_interpret::CacheKey::cache_key(&**t3)
                        ^ u128::from(g.char_code()).rotate_left(64);
                    if self.out.type3_probes.len() < TYPE3_PROBES && self.type3_probed.insert(key) {
                        let shape = type3_shape(t3, &props.paint);
                        if !shape.is_empty() {
                            self.out.type3_probes.push((shape, advance_units));
                        }
                    }
                    None
                }
                _ => None,
            };
            let drawn_type3 = matches!(&**g, Glyph::Type3(_));
            // Clipped away: its middle is outside the clip.
            let middle = Point::new((origin.x + end.x) / 2.0, origin.y - 0.3 * size as f64);
            if self
                .clips
                .last()
                .is_some_and(|c| !c.inflate(0.5, 0.5).contains(middle))
            {
                continue;
            }
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
                type3,
                drawn_type3,
            });
        }
    }
}

/// The filled outline of a Type 3 glyph, in 1000-unit ems with y up. Drawn
/// with a [`Collector`] in capture mode rather than a device of its own: the
/// interpreter is generic over the device, and a new device type would
/// compile a second copy of it (about 1.5 MB).
fn type3_shape<'a>(
    glyph: &hayro::hayro_interpret::font::Type3Glyph<'a>,
    paint: &hayro::hayro_interpret::Paint<'a>,
) -> crate::shape::Shape {
    let mut d = Collector::new();
    d.capture = Some(BezPath::new());
    glyph.interpret(&mut d, Affine::IDENTITY, Affine::IDENTITY, paint);
    crate::shape::Shape::of(&d.capture.unwrap_or_default())
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
        crate::interpreter_settings(),
    );
    let mut collector = Collector::new();

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
