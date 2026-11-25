//! Drawing laid-out pages into pixels.
//!
//! Uses the same CPU rasterizer (vello_cpu) the PDF path uses, so the engine
//! ships one rasterizer. Glyph outlines are cached per font and glyph in
//! font units, under a byte budget, and drawn at any size by transform.

use std::sync::Arc;

use akkhor_core::{Error, LruCache, Result, Theme};
use harfrust::GlyphId;
use harfrust::font::glyph::outline::{DrawMemory, DrawOptions, PathSink};
use kurbo::{Affine, BezPath};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};

use crate::fonts::{FontId, FontSet};
use crate::layout::Page;

/// Outline cache shared across pages of a book.
pub struct GlyphCache {
    outlines: LruCache<(FontId, u32), Option<Arc<BezPath>>>,
}

impl GlyphCache {
    /// A cache holding at most `budget` bytes of outlines.
    pub fn new(budget: usize) -> Self {
        Self {
            outlines: LruCache::new(budget),
        }
    }

    fn outline(&mut self, fonts: &FontSet, font: FontId, glyph: u32) -> Option<Arc<BezPath>> {
        self.outlines
            .get_or_insert_with((font, glyph), || {
                let path = fonts.get(font).and_then(|f| {
                    let outlines = f.font.outline_glyphs();
                    let g = outlines.get(GlyphId::new(glyph))?;
                    let mut sink = BezSink(BezPath::new());
                    g.draw(&DrawOptions::new(), DrawMemory::Scratch, &mut sink)
                        .ok()?;
                    Some(Arc::new(sink.0))
                });
                let cost = path.as_ref().map_or(16, |p| 48 + p.elements().len() * 40);
                (path, cost)
            })
            .clone()
    }

    pub fn used_bytes(&self) -> usize {
        self.outlines.used_bytes()
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new(4 << 20)
    }
}

struct BezSink(BezPath);

impl PathSink for BezSink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0
            .quad_to((cx0 as f64, cy0 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (cx0 as f64, cy0 as f64),
            (cx1 as f64, cy1 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

fn color(c: akkhor_core::Rgba) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Draws `page` into `out`: RGBA8 premultiplied, `width * height * 4` bytes,
/// rows tightly packed (the layout of an Android `ARGB_8888` bitmap).
pub fn render_page(
    page: &Page,
    fonts: &FontSet,
    cache: &mut GlyphCache,
    theme: &Theme,
    width: u32,
    height: u32,
    out: &mut [u8],
) -> Result<()> {
    let needed = width as usize * height as usize * 4;
    if out.len() < needed {
        return Err(Error::BufferTooSmall {
            needed,
            got: out.len(),
        });
    }
    let (w, h) = (
        u16::try_from(width).map_err(|_| Error::malformed("page too wide"))?,
        u16::try_from(height).map_err(|_| Error::malformed("page too tall"))?,
    );
    let mut ctx = RenderContext::new(w, h);
    ctx.set_paint(color(theme.text));
    for line in &page.lines {
        for g in &line.glyphs {
            let Some(font) = fonts.get(g.font) else {
                continue;
            };
            let Some(path) = cache.outline(fonts, g.font, g.glyph) else {
                continue;
            };
            let s = (g.size / font.units_per_em) as f64;
            // Font units are y-up; pixels are y-down.
            ctx.set_transform(
                Affine::translate((g.x as f64, g.y as f64)) * Affine::scale_non_uniform(s, -s),
            );
            ctx.fill_path(&path);
        }
    }
    ctx.flush();
    let target = PixmapMut::new(w, h, &mut out[..needed])
        .ok_or_else(|| Error::malformed("bad target size"))?;
    ctx.render_with(
        target,
        &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(color(theme.background)),
            ..Default::default()
        },
    );
    Ok(())
}
