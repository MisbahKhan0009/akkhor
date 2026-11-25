//! PDF support for Akkhor.
//!
//! Opening never reads the whole file: the PDF is memory-mapped, the
//! cross-reference table at the end of the file is read first, and objects
//! are parsed only when a page asks for them.
//!
//! Rendering works on tiles. The caller asks for any rectangle of a page at
//! any zoom and the engine draws straight into the caller's pixel buffer (on
//! Android, the bitmap's own memory), so a zoomed-in page never needs a
//! full-page bitmap.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub mod glyphs;
mod read;
pub mod recover;
pub mod reference;
pub mod text;
mod theme;

pub use reference::register_reference_font;

pub use glyphs::{PageGlyphs, PdfFont, PdfGlyph};
pub use text::{PageText, TextSpan};

use akkhor_core::{Error, Result, Source, Theme, Tile};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::AlphaColor;
use hayro::vello_cpu::color::Srgb;
use hayro::vello_cpu::{RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

pub use hayro::vello_cpu::PixmapMut;

/// An open PDF.
pub struct PdfDocument {
    pdf: Pdf,
    interpreter: InterpreterSettings,
    /// Glyph-to-letter tables per embedded font (see [`recover`]).
    recoveries: Mutex<HashMap<u128, Option<Arc<recover::Recovery>>>>,
    /// Text of recently used pages, most recent last.
    texts: Mutex<Vec<(usize, Arc<PageText>)>>,
    /// Where colour images land on recently themed pages.
    image_boxes: Mutex<Vec<(usize, Arc<theme::ImageBoxes>)>>,
}

/// Pages whose image boxes are kept for theming (a few KB each).
const IMAGE_BOX_PAGES: usize = 16;

/// Fonts per page whose glyphs are recovered; a page with more (a hostile
/// file) reads the rest from its text layer.
const MAX_RECOVERED_FONTS: usize = 48;

/// Pages whose text [`PdfDocument::page_text_cached`] keeps.
const TEXT_CACHE_PAGES: usize = 4;

/// A search hit in a PDF.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfMatch {
    pub page: usize,
    /// Byte range in that page's [`PageText::text`].
    pub range: std::ops::Range<usize>,
    /// Highlight boxes, in points (top-left origin), one per line.
    pub rects: Vec<hayro::kurbo::Rect>,
}

impl PdfDocument {
    /// Opens a PDF from a memory-mapped source.
    pub fn open(source: &Source) -> Result<Self> {
        source.advise_random();
        let data: Arc<akkhor_core::Bytes> = source.shared();
        let pdf = contain(|| Pdf::new(data))?.map_err(|e| match e {
            LoadPdfError::Decryption(_) => Error::Protected,
            LoadPdfError::Invalid => Error::malformed("unreadable PDF structure"),
        })?;
        Ok(Self {
            pdf,
            interpreter: InterpreterSettings::default(),
            recoveries: Mutex::new(HashMap::new()),
            texts: Mutex::new(Vec::new()),
            image_boxes: Mutex::new(Vec::new()),
        })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Page size in PDF points (1/72 inch), after rotation and crop box.
    pub fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        let page = self.pdf.pages().get(index).ok_or(Error::OutOfRange)?;
        Ok(page.render_dimensions())
    }

    /// The glyphs drawn on a page, with fonts, positions (points, top-left
    /// origin) and whatever the PDF's own text layer says they mean.
    pub fn glyphs(&self, index: usize) -> Result<PageGlyphs> {
        let page = self.pdf.pages().get(index).ok_or(Error::OutOfRange)?;
        contain(|| glyphs::collect(page))
    }

    /// The page's text exactly as the PDF's own text layer (`ToUnicode`)
    /// gives it, with no repair. Useful for comparison; use
    /// [`page_text`](Self::page_text) for reading, search and copy.
    pub fn page_text_raw(&self, index: usize) -> Result<PageText> {
        let glyphs = self.glyphs(index)?;
        Ok(text::assemble(
            &glyphs,
            |word| {
                word.iter()
                    .map(|g| {
                        let t = g
                            .unicode
                            .clone()
                            .filter(|u| text::usable(u))
                            .unwrap_or_default();
                        text::GlyphPiece::new(g, t, recover::Role::Base)
                    })
                    .collect()
            },
            |line| line,
        ))
    }

    /// The page's text, repaired for reading, search and copy.
    ///
    /// For every embedded font the letters of each glyph are recovered by
    /// running the font's own shaping rules (see [`recover`]); that fixes
    /// conjuncts and rephs the PDF's text layer lost, and works even when
    /// the PDF has no text layer at all. Glyphs are then put back into
    /// reading order: vowel signs drawn before their consonant, rephs drawn
    /// after their syllable and two-part vowels.
    pub fn page_text(&self, index: usize) -> Result<PageText> {
        let glyphs = self.glyphs(index)?;
        let tables = glyphs
            .fonts
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if i < MAX_RECOVERED_FONTS {
                    self.recovery(f)
                } else {
                    None
                }
            })
            .collect();
        let fonts = read::readings(&glyphs, tables);
        contain(|| {
            text::assemble(
                &glyphs,
                |word| read::word_pieces(word, &fonts),
                text::reorder,
            )
        })
    }

    /// [`page_text`](Self::page_text), kept for the last few pages: hit
    /// testing, selection and copy on one page reuse one extraction.
    pub fn page_text_cached(&self, index: usize) -> Result<Arc<PageText>> {
        {
            let cache = self.texts.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, t)) = cache.iter().find(|(i, _)| *i == index) {
                return Ok(t.clone());
            }
        }
        let text = Arc::new(self.page_text(index)?);
        let mut cache = self.texts.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= TEXT_CACHE_PAGES {
            cache.remove(0);
        }
        cache.push((index, text.clone()));
        Ok(text)
    }

    /// Searches the document page by page from `first_page`, wrapping
    /// around. Lazy: a page's text is only extracted when the iterator
    /// reaches it, so the caller stops whenever it has enough hits. Pages
    /// that fail to load are skipped.
    pub fn search<'a>(
        &'a self,
        query: &str,
        first_page: usize,
    ) -> impl Iterator<Item = PdfMatch> + 'a {
        let query = query.to_string();
        let n = self.page_count();
        (0..n).flat_map(move |k| {
            let page = (first_page + k) % n;
            let hits: Vec<PdfMatch> = match self.page_text(page) {
                Ok(text) => text
                    .find(&query)
                    .into_iter()
                    .map(|range| PdfMatch {
                        page,
                        rects: text.rects(range.clone()),
                        range,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            hits
        })
    }

    /// The recovery table for a font, built once per document.
    fn recovery(&self, font: &PdfFont) -> Option<Arc<recover::Recovery>> {
        let data = font.data.as_ref()?;
        let mut cache = self.recoveries.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .entry(font.key)
            .or_insert_with(|| {
                contain(|| reference::recover(data, font.name.as_deref()))
                    .ok()
                    .flatten()
                    .map(Arc::new)
            })
            .clone()
    }

    /// Pixel size of a whole page at `scale` (pixels per point).
    pub fn page_pixels(&self, index: usize, scale: f32) -> Result<(u32, u32)> {
        let (w, h) = self.page_size(index)?;
        Ok(((w * scale).ceil() as u32, (h * scale).ceil() as u32))
    }

    /// Draws one tile of a page into `out`, an RGBA8 premultiplied buffer of
    /// exactly `tile.width * tile.height * 4` bytes, with rows tightly packed.
    ///
    /// `scale` is pixels per PDF point; `tile` is in pixels of the page at
    /// that scale. The background is filled with the theme's background.
    pub fn render_tile(
        &self,
        index: usize,
        scale: f32,
        tile: Tile,
        theme: &Theme,
        out: &mut [u8],
    ) -> Result<()> {
        let needed = tile.rgba_len();
        if out.len() < needed {
            return Err(Error::BufferTooSmall {
                needed,
                got: out.len(),
            });
        }
        if tile.width == 0 || tile.height == 0 {
            return Ok(());
        }
        let (w, h) = (to_u16(tile.width)?, to_u16(tile.height)?);
        // The PDF interpreter is young; a panic on a malformed page becomes an
        // error instead of unwinding into the caller.
        contain(|| self.draw_tile(index, scale, tile, theme, w, h, &mut out[..needed]))?
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tile(
        &self,
        index: usize,
        scale: f32,
        tile: Tile,
        theme: &Theme,
        w: u16,
        h: u16,
        out: &mut [u8],
    ) -> Result<()> {
        let page = self.pdf.pages().get(index).ok_or(Error::OutOfRange)?;

        // Move the page so the tile's corner lands at the origin, then scale.
        let transform = Affine::translate((-(tile.x as f64), -(tile.y as f64)))
            * Affine::scale(scale as f64)
            * to_affine(page.initial_transform(true));

        let cache = RenderCache::new();
        let mut ctx = RenderContext::new(w, h);
        render_into(
            page,
            &cache,
            &self.interpreter,
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();

        // Drawn on white, as the PDF was designed; the theme is applied after.
        let target =
            PixmapMut::new(w, h, &mut *out).ok_or_else(|| Error::malformed("bad target size"))?;
        ctx.render_with(
            target,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(AlphaColor::<Srgb>::from_rgba8(255, 255, 255, 255)),
                ..Default::default()
            },
        );
        if !theme::is_original(theme) {
            let boxes = self.image_boxes(index, page);
            theme::apply(out, tile, scale, &boxes, theme);
        }
        Ok(())
    }

    /// Where colour images land on a page (cached for recent pages).
    fn image_boxes(
        &self,
        index: usize,
        page: &hayro::hayro_syntax::page::Page<'_>,
    ) -> Arc<theme::ImageBoxes> {
        {
            let cache = self.image_boxes.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, b)) = cache.iter().find(|(i, _)| *i == index) {
                return b.clone();
            }
        }
        let boxes = Arc::new(theme::image_boxes(page));
        let mut cache = self.image_boxes.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= IMAGE_BOX_PAGES {
            cache.remove(0);
        }
        cache.push((index, boxes.clone()));
        boxes
    }

    /// Convenience: renders a whole page and returns its pixels.
    pub fn render_page(&self, index: usize, scale: f32, theme: &Theme) -> Result<RenderedPage> {
        let (w, h) = self.page_pixels(index, scale)?;
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        self.render_tile(index, scale, Tile::new(0, 0, w, h), theme, &mut pixels)?;
        Ok(RenderedPage {
            width: w,
            height: h,
            pixels,
        })
    }
}

/// A rendered page: RGBA8, premultiplied alpha, rows tightly packed.
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RenderedPage {
    /// Straight-alpha RGBA, e.g. for PNG encoding.
    pub fn into_unpremultiplied(self) -> Vec<u8> {
        let mut px = self.pixels;
        for p in px.as_chunks_mut::<4>().0 {
            let a = p[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        px
    }
}

/// Runs `f`, turning a panic into an error.
fn contain<T>(f: impl FnOnce() -> T) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .map_err(|_| Error::malformed("the PDF interpreter failed on this file"))
}

fn to_u16(v: u32) -> Result<u16> {
    u16::try_from(v).map_err(|_| Error::malformed("tile larger than 65535 px"))
}

fn to_affine<T: hayro::hayro_interpret::util::TransformExt>(t: T) -> Affine {
    t.to_kurbo()
}

/// Whether a font name is one of the Bijoy ("MJ") fonts, whose text is
/// Latin-1 that only looks like Bangla: SutonnyMJ, SolaimanLipiMJ,
/// "Sutonny MJ Bold", ... The "OMJ" fonts (SutonnyOMJ) are their Unicode
/// versions and are not Bijoy.
pub fn is_bijoy_font(name: &str) -> bool {
    let base: String = name
        .split([',', '-', '+'])
        .find(|p| !p.is_empty() && !(p.len() == 6 && p.chars().all(|c| c.is_ascii_uppercase())))
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    (base.ends_with("MJ") && !base.ends_with("OMJ")) || base.contains("BIJOY")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str) -> PdfDocument {
        // Reference fonts, whether or not `bundled-fonts` is on.
        static FONTS: std::sync::Once = std::sync::Once::new();
        FONTS.call_once(|| {
            for f in [
                "NotoSerifBengali-Regular.ttf",
                "NotoSerifDevanagari-Regular.ttf",
            ] {
                let path = format!("{}/../../fonts/{f}", env!("CARGO_MANIFEST_DIR"));
                register_reference_font(Arc::new(std::fs::read(path).unwrap()));
            }
        });
        let path = format!("{}/../../tests/{name}", env!("CARGO_MANIFEST_DIR"));
        PdfDocument::open(&Source::open(path).unwrap()).unwrap()
    }

    #[test]
    fn search_finds_words_the_text_layer_breaks() {
        let pdf = open("corpus/pdf/bn.chrome.raw.pdf");
        let raw = pdf.page_text_raw(0).unwrap();
        assert!(raw.find("যুক্তাক্ষরের").is_empty());
        let hits: Vec<PdfMatch> = pdf.search("যুক্তাক্ষরের", 0).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page, 0);
        assert!(!hits[0].rects.is_empty());
        // Lazy: taking one hit does not need the rest of the document.
        assert_eq!(pdf.search("কর", 0).take(1).count(), 1);
    }

    #[test]
    fn selection_snaps_to_syllables_and_words() {
        let pdf = open("corpus/pdf/bn.chrome.pdf");
        let text = pdf.page_text_cached(0).unwrap();
        let at = text.text.find("যুক্তাক্ষরের").unwrap();
        // Inside the conjunct ক্ত: snaps back to its start.
        let inside = at + "যুক্".len();
        assert_eq!(text.snap(inside), at + "যু".len());
        let word = text.word_at(inside);
        assert_eq!(&text.text[word], "যুক্তাক্ষরের");
        // Hit-testing the word's box lands in the word.
        let r = text.rects(at..at + "যুক্তাক্ষরের".len())[0];
        let hit = text
            .offset_at((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)
            .unwrap();
        assert!(hit >= at && hit <= at + "যুক্তাক্ষরের".len());
        // The cache returns the same extraction.
        assert!(Arc::ptr_eq(&text, &pdf.page_text_cached(0).unwrap()));
    }

    /// A one-page PDF: a 2×2 RGB image (all green) at (50, 50, 100×100)
    /// and a black square at (200, 50, 100×100), on a 400×200 page.
    fn image_pdf(colour_space: &str, pixels: &[u8]) -> Vec<u8> {
        let content = "q 100 0 0 100 50 50 cm /Im Do Q 0 g 200 50 100 100 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Contents 4 0 R /Resources << /XObject << /Im 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        offsets.push(out.len());
        out.extend(format!("5 0 obj\n<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /{colour_space} /BitsPerComponent 8 /Length {} >>\nstream\n", pixels.len()).as_bytes());
        out.extend(pixels);
        out.extend(b"\nendstream\nendobj\n");
        let xref = out.len();
        out.extend(b"xref\n0 6\n0000000000 65535 f \n");
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }

    fn pixel_at(page: &RenderedPage, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * page.width + x) * 4) as usize;
        [page.pixels[i], page.pixels[i + 1], page.pixels[i + 2]]
    }

    #[test]
    fn themes_recolour_text_but_not_colour_images() {
        let green = [0u8, 200, 0].repeat(4);
        let pdf = PdfDocument::open(&Source::from_vec(image_pdf("DeviceRGB", &green))).unwrap();
        let t = Theme::NIGHT;
        let page = pdf.render_page(0, 1.0, &t).unwrap();
        // Top-left origin: the PDF's y = 50..150 is rows 50..150 here too.
        assert_eq!(
            pixel_at(&page, 100, 100),
            [0, 200, 0],
            "image keeps its colour"
        );
        assert_eq!(
            pixel_at(&page, 250, 100),
            [t.text.r, t.text.g, t.text.b],
            "black becomes text colour"
        );
        assert_eq!(
            pixel_at(&page, 350, 20),
            [t.background.r, t.background.g, t.background.b]
        );
        // The original theme changes nothing.
        let page = pdf.render_page(0, 1.0, &Theme::ORIGINAL).unwrap();
        assert_eq!(pixel_at(&page, 250, 100), [0, 0, 0]);
        assert_eq!(pixel_at(&page, 350, 20), [255, 255, 255]);
    }

    #[test]
    fn greyscale_images_are_themed_like_scanned_text() {
        let white = [255u8; 4];
        let pdf = PdfDocument::open(&Source::from_vec(image_pdf("DeviceGray", &white))).unwrap();
        let t = Theme::NIGHT;
        let page = pdf.render_page(0, 1.0, &t).unwrap();
        assert_eq!(
            pixel_at(&page, 100, 100),
            [t.background.r, t.background.g, t.background.b]
        );
    }

    #[test]
    fn recognizes_bijoy_fonts() {
        for name in [
            "SutonnyMJ",
            "SolaimanLipiMJ,Italic",
            "Sutonny MJ",
            "BijoyBaijayanta",
        ] {
            assert!(is_bijoy_font(name), "{name}");
        }
        for name in [
            "NotoSerifBengali-Regular",
            "Kalpurush",
            "TimesNewRomanPSMT",
            "MJollnir",
            "SutonnyOMJ",
            "SutonnyOMJ-Bold",
        ] {
            assert!(!is_bijoy_font(name), "{name}");
        }
    }

    #[test]
    fn collects_glyphs_with_positions_and_fonts() {
        let pdf = open("corpus/pdf/bn.chrome.pdf");
        let (w, h) = pdf.page_size(0).unwrap();
        let page = pdf.glyphs(0).unwrap();
        assert!(page.glyphs.len() > 300, "{} glyphs", page.glyphs.len());
        assert!(
            page.fonts
                .iter()
                .any(|f| f.name.as_deref() == Some("NotoSerifBengali-Regular"))
        );
        for g in &page.glyphs {
            assert!(g.origin.0 >= 0.0 && g.origin.0 <= w && g.origin.1 >= 0.0 && g.origin.1 <= h);
            assert!((10.0..30.0).contains(&g.size), "size {}", g.size);
        }
        // Text runs top to bottom.
        let first = page.glyphs.first().unwrap().origin.1;
        let last = page.glyphs.last().unwrap().origin.1;
        assert!(first < last);
    }
}
