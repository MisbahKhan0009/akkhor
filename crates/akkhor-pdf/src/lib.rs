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

use std::sync::Arc;

pub mod glyphs;

pub use glyphs::{PageGlyphs, PdfFont, PdfGlyph};

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

        let target =
            PixmapMut::new(w, h, out).ok_or_else(|| Error::malformed("bad target size"))?;
        let bg = theme.background;
        ctx.render_with(
            target,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(AlphaColor::<Srgb>::from_rgba8(
                    bg.r, bg.g, bg.b, bg.a,
                )),
                ..Default::default()
            },
        );
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str) -> PdfDocument {
        let path = format!("{}/../../tests/{name}", env!("CARGO_MANIFEST_DIR"));
        PdfDocument::open(&Source::open(path).unwrap()).unwrap()
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
