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
        let pdf = Pdf::new(data).map_err(|e| match e {
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

        let target = PixmapMut::new(w, h, &mut out[..needed])
            .ok_or_else(|| Error::malformed("bad target size"))?;
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
        for p in px.chunks_exact_mut(4) {
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

fn to_u16(v: u32) -> Result<u16> {
    u16::try_from(v).map_err(|_| Error::malformed("tile larger than 65535 px"))
}

fn to_affine<T: hayro::hayro_interpret::util::TransformExt>(t: T) -> Affine {
    t.to_kurbo()
}
