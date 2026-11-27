//! Drawing laid-out pages into pixels.
//!
//! Uses the same CPU rasterizer (vello_cpu) the PDF path uses, so the engine
//! ships one rasterizer. Glyph outlines are cached per font and glyph in
//! font units, under a byte budget. Text up to [`MASK_MAX_SIZE`] pixels is
//! drawn from coverage masks cached per font, glyph, size, quarter-pixel
//! position and synthetic style, blended straight into the target; larger
//! text is filled from its outline.

use std::sync::Arc;

use akkhor_core::{Error, LruCache, Result, Theme};
use harfrust::GlyphId;
use harfrust::font::glyph::outline::{DrawMemory, DrawOptions, PathSink};
use kurbo::{Affine, BezPath, Shape as _, Stroke};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::peniko::ImageSampler;
use vello_cpu::{
    Image, ImageSource, PixelMetadata, Pixmap, PixmapMut, RasterizerSettings, RenderContext,
    Resources, TargetInit,
};

use crate::fonts::{FontId, FontSet};
use crate::image::{ImageProvider, resize};
use crate::layout::{Page, PlacedGlyph};

/// Drawing caches shared across the pages of a book: glyph outlines and
/// decoded images, each under its own byte budget.
pub struct GlyphCache {
    outlines: LruCache<(FontId, u32), Option<Arc<BezPath>>>,
    masks: LruCache<MaskKey, Option<Arc<Mask>>>,
    images: LruCache<(String, u32, u32), Option<Arc<Pixmap>>>,
    /// Masks on (the default); off fills every glyph from its outline.
    pub use_masks: bool,
    /// One rasteriser kept for making masks: resizing it reuses its buffers.
    raster: Option<(RenderContext, Resources)>,
}

/// Text larger than this, in pixels, is filled from outlines: its masks
/// would be big and rarely reused.
pub const MASK_MAX_SIZE: f32 = 160.0;

/// Steps per pixel for a mask's position. A quarter pixel is below what
/// the eye sees in body text and keeps 16 variants per glyph at most.
const SUB: f32 = 4.0;

/// Font, glyph, size (bits), position step across and down, style bits.
type MaskKey = (FontId, u32, u32, u8, u8, u8);

/// A glyph's coverage, 0 to 255 per pixel, and where its top-left pixel
/// sits relative to the glyph's whole-pixel origin.
struct Mask {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
    /// Per row, the columns from the first to past the last inked pixel.
    spans: Vec<(u16, u16)>,
}

impl GlyphCache {
    /// A cache holding at most `budget` bytes of outlines and four times that
    /// of decoded images.
    pub fn new(budget: usize) -> Self {
        Self {
            outlines: LruCache::new(budget),
            masks: LruCache::new(budget),
            images: LruCache::new(budget * 4),
            use_masks: true,
            raster: None,
        }
    }

    /// Decodes `src` scaled to `w` x `h`, or returns it from the cache.
    fn image(
        &mut self,
        provider: &dyn ImageProvider,
        src: &str,
        w: u32,
        h: u32,
    ) -> Option<Arc<Pixmap>> {
        self.images
            .get_or_insert_with((src.to_string(), w, h), || {
                let pixmap = provider.decoded(src, w, h).and_then(|img| {
                    let (pw, ph) = (u16::try_from(w).ok()?, u16::try_from(h).ok()?);
                    let rgba = resize(&img, w, h);
                    Some(Arc::new(Pixmap::from_parts(
                        rgba,
                        pw,
                        ph,
                        PixelMetadata {
                            may_have_transparency: true,
                            ..Default::default()
                        },
                    )))
                });
                let cost = (w as usize * h as usize * 4).max(16);
                (pixmap, cost)
            })
            .clone()
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

    /// The glyph's coverage mask at `size` px and position step
    /// (`sx`, `sy`) out of [`SUB`], made on first use.
    fn mask(&mut self, fonts: &FontSet, g: &PlacedGlyph, sx: u8, sy: u8) -> Option<Arc<Mask>> {
        let style = u8::from(g.synthetic.bold) | (u8::from(g.synthetic.italic) << 1);
        let key = (g.font, g.glyph, g.size.to_bits(), sx, sy, style);
        if let Some(m) = self.masks.get(&key) {
            return m.clone();
        }
        let path = self.outline(fonts, g.font, g.glyph);
        let upem = fonts.get(g.font).map(|f| f.units_per_em);
        let mask = match (path, upem) {
            (Some(path), Some(upem)) => draw_mask(
                &mut self.raster,
                &path,
                (g.size / upem) as f64,
                upem as f64,
                f64::from(sx) / f64::from(SUB),
                f64::from(sy) / f64::from(SUB),
                g.synthetic,
            )
            .map(Arc::new),
            _ => None,
        };
        let cost = mask
            .as_ref()
            .map_or(16, |m| 72 + m.alpha.len() + m.spans.len() * 4);
        self.masks.get_or_insert_with(key, || (mask, cost)).clone()
    }

    pub fn used_bytes(&self) -> usize {
        self.outlines.used_bytes() + self.masks.used_bytes()
    }

    /// Frees everything (outlines, masks, pictures, the mask rasteriser).
    pub fn clear(&mut self) {
        self.outlines.clear();
        self.masks.clear();
        self.images.clear();
        self.raster = None;
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new(4 << 20)
    }
}

/// Glyph transform: font units (y-up) to pixels (y-down) at scale `s`,
/// with synthetic italic slanting by 12 degrees (tan 12° ≈ 0.21).
fn glyph_transform(x: f64, y: f64, s: f64, italic: bool) -> Affine {
    let mut t = Affine::translate((x, y)) * Affine::scale_non_uniform(s, -s);
    if italic {
        t *= Affine::new([1.0, 0.0, 0.21, 1.0, 0.0, 0.0]);
    }
    t
}

/// Synthetic bold thickens every stroke by about 3% of the em.
fn bold_stroke(upem: f64) -> Stroke {
    Stroke::new(upem * 0.03)
}

/// Rasterises one glyph, placed at (`fx`, `fy`) within its first pixel,
/// into a coverage mask. None for empty glyphs (spaces).
fn draw_mask(
    raster: &mut Option<(RenderContext, Resources)>,
    path: &BezPath,
    s: f64,
    upem: f64,
    fx: f64,
    fy: f64,
    synthetic: crate::fonts::Synthetic,
) -> Option<Mask> {
    if path.elements().is_empty() {
        return None;
    }
    let t = glyph_transform(fx, fy, s, synthetic.italic);
    let mut bbox = t.transform_rect_bbox(path.bounding_box());
    if synthetic.bold {
        bbox = bbox.inflate(upem * 0.015 * s + 0.5, upem * 0.015 * s + 0.5);
    }
    let (left, top) = (bbox.x0.floor() as i32 - 1, bbox.y0.floor() as i32 - 1);
    let (right, bottom) = (bbox.x1.ceil() as i32 + 1, bbox.y1.ceil() as i32 + 1);
    let (w, h) = (
        u16::try_from(right - left).ok()?,
        u16::try_from(bottom - top).ok()?,
    );
    if w == 0 || h == 0 {
        return None;
    }
    let (ctx, resources) = match raster {
        Some((ctx, res)) => {
            ctx.reset_and_resize(w, h);
            (ctx, res)
        }
        None => {
            let (ctx, res) = raster.insert((RenderContext::new(w, h), Resources::default()));
            (ctx, res)
        }
    };
    ctx.set_paint(AlphaColor::<Srgb>::WHITE);
    ctx.set_transform(Affine::translate((-f64::from(left), -f64::from(top))) * t);
    ctx.fill_path(path);
    if synthetic.bold {
        ctx.set_stroke(bold_stroke(upem));
        ctx.stroke_path(path);
    }
    ctx.flush();
    let mut rgba = vec![0u8; usize::from(w) * usize::from(h) * 4];
    let target = PixmapMut::new(w, h, &mut rgba)?;
    ctx.render_with(
        target,
        resources,
        RasterizerSettings {
            target_init: TargetInit::Clear(AlphaColor::TRANSPARENT),
            ..Default::default()
        },
    );
    let alpha: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
    if alpha.iter().all(|&a| a == 0) {
        return None;
    }
    let spans = alpha
        .chunks_exact(usize::from(w))
        .map(|row| {
            let first = row.iter().position(|&a| a != 0).unwrap_or(0);
            let end = row.iter().rposition(|&a| a != 0).map_or(first, |i| i + 1);
            (first as u16, end as u16)
        })
        .collect();
    Some(Mask {
        left,
        top,
        width: u32::from(w),
        height: u32::from(h),
        alpha,
        spans,
    })
}

/// x * y / 255, rounded, for bytes.
#[inline]
fn mul255(x: u32, y: u32) -> u32 {
    let v = x * y + 128;
    (v + (v >> 8)) >> 8
}

/// Blends `mask` in colour `c` (premultiplied) at whole pixel (`x`, `y`)
/// over `out` (premultiplied RGBA, `width` x `height`), source over.
fn blend_mask(out: &mut [u8], width: u32, height: u32, mask: &Mask, x: i32, y: i32, c: [u32; 4]) {
    let x0 = x + mask.left;
    let y0 = y + mask.top;
    let cx0 = x0.max(0);
    let cy0 = y0.max(0);
    let cx1 = (x0 + mask.width as i32).min(width as i32);
    let cy1 = (y0 + mask.height as i32).min(height as i32);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let mw = mask.width as usize;
    for py in cy0..cy1 {
        let (first, end) = mask.spans[(py - y0) as usize];
        let (sx0, sx1) = (cx0.max(x0 + i32::from(first)), cx1.min(x0 + i32::from(end)));
        let mrow = (py - y0) as usize * mw;
        let orow = py as usize * width as usize * 4;
        for px in sx0..sx1 {
            let a = u32::from(mask.alpha[mrow + (px - x0) as usize]);
            if a == 0 {
                continue;
            }
            let o = orow + px as usize * 4;
            let p = &mut out[o..o + 4];
            if a == 255 && c[3] == 255 {
                p.copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
                continue;
            }
            let sa = mul255(c[3], a);
            let keep = 255 - sa;
            for i in 0..3 {
                p[i] = (mul255(c[i], a) + mul255(u32::from(p[i]), keep)).min(255) as u8;
            }
            p[3] = (sa + mul255(u32::from(p[3]), keep)).min(255) as u8;
        }
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
/// `images` supplies the pictures of image blocks; without it they are left
/// blank.
#[allow(clippy::too_many_arguments)]
pub fn render_page(
    page: &Page,
    fonts: &FontSet,
    images: Option<&dyn ImageProvider>,
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
    // Whether anything goes through the rasteriser; if not, the background
    // is filled directly, which is several times cheaper.
    let mut vector = false;
    if let Some(provider) = images {
        for img in &page.images {
            let (iw, ih) = (
                img.width.round().max(1.0) as u32,
                img.height.round().max(1.0) as u32,
            );
            let Some(pixmap) = cache.image(provider, &img.src, iw, ih) else {
                continue;
            };
            ctx.set_transform(Affine::translate((
                img.x.round() as f64,
                img.y.round() as f64,
            )));
            ctx.set_paint(Image {
                image: ImageSource::Pixmap(pixmap),
                sampler: ImageSampler::default(),
            });
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, iw as f64, ih as f64));
            vector = true;
        }
    }
    ctx.set_paint(color(theme.text));
    // Small text is blended from cached masks once the background and the
    // pictures are drawn; text of one colour blends the same in any order.
    let mut masked: Vec<(Arc<Mask>, i32, i32)> = Vec::new();
    for line in &page.lines {
        for g in &line.glyphs {
            let Some(font) = fonts.get(g.font) else {
                continue;
            };
            if cache.use_masks && g.size <= MASK_MAX_SIZE && g.x.is_finite() && g.y.is_finite() {
                let qx = (g.x * SUB).round();
                let qy = (g.y * SUB).round();
                let (ix, iy) = ((qx / SUB).floor(), (qy / SUB).floor());
                let sx = (qx - ix * SUB) as u8;
                let sy = (qy - iy * SUB) as u8;
                if let Some(m) = cache.mask(fonts, g, sx, sy) {
                    masked.push((m, ix as i32, iy as i32));
                }
                continue;
            }
            let Some(path) = cache.outline(fonts, g.font, g.glyph) else {
                continue;
            };
            vector = true;
            let upem = font.units_per_em as f64;
            let s = g.size as f64 / upem;
            ctx.set_transform(glyph_transform(
                g.x as f64,
                g.y as f64,
                s,
                g.synthetic.italic,
            ));
            ctx.fill_path(&path);
            if g.synthetic.bold {
                ctx.set_stroke(bold_stroke(upem));
                ctx.stroke_path(&path);
            }
        }
    }
    if vector {
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
    } else {
        let b = theme.background;
        let a = u32::from(b.a);
        let px = [
            mul255(u32::from(b.r), a) as u8,
            mul255(u32::from(b.g), a) as u8,
            mul255(u32::from(b.b), a) as u8,
            b.a,
        ];
        let row = width as usize * 4;
        for chunk in out[..row].as_chunks_mut::<4>().0 {
            chunk.copy_from_slice(&px);
        }
        let (first, rest) = out[..needed].split_at_mut(row);
        for r in rest.chunks_exact_mut(row) {
            r.copy_from_slice(first);
        }
    }
    if !masked.is_empty() {
        let t = theme.text;
        let a = u32::from(t.a);
        let c = [
            mul255(u32::from(t.r), a),
            mul255(u32::from(t.g), a),
            mul255(u32::from(t.b), a),
            a,
        ];
        for (m, x, y) in &masked {
            blend_mask(&mut out[..needed], width, height, m, *x, *y, c);
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;
    use crate::layout::{LayoutOptions, paginate};
    use crate::model::Document;

    const W: u32 = 600;
    const H: u32 = 900;

    fn page(html: &str, size: f32) -> Page {
        let doc = Document::from_html(html);
        let opts = LayoutOptions {
            width: W as f32,
            height: H as f32,
            font_size: size,
            margin_x: 30.0,
            ..Default::default()
        };
        paginate(&doc, &opts, &FontSet::bundled()).remove(0)
    }

    fn draw(p: &Page, masks: bool, theme: &Theme) -> (Vec<u8>, GlyphCache) {
        let mut cache = GlyphCache::new(1 << 20);
        cache.use_masks = masks;
        let mut px = vec![0u8; (W * H * 4) as usize];
        render_page(
            p,
            &FontSet::bundled(),
            None,
            &mut cache,
            theme,
            W,
            H,
            &mut px,
        )
        .unwrap();
        (px, cache)
    }

    /// Largest channel difference and the mean one.
    fn diff(a: &[u8], b: &[u8]) -> (u8, f64) {
        let max = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        let sum: u64 = a
            .iter()
            .zip(b)
            .map(|(x, y)| u64::from(x.abs_diff(*y)))
            .sum();
        (max, sum as f64 / a.len() as f64)
    }

    #[test]
    fn masks_draw_what_outlines_draw() {
        let html = "<p>The quick brown fox, আমার সোনার বাংলা, नमस्ते दुनिया.</p>\
                    <p><b>Bold</b> <i>italic</i> <b><i>both</i></b> ক্ষ্ম ন্ত্র</p>";
        let p = page(html, 26.0);
        let mut theme = Theme::PAPER;
        for alpha in [255u8, 160] {
            theme.text.a = alpha;
            let (a, _) = draw(&p, false, &theme);
            let (b, cache) = draw(&p, true, &theme);
            let (max, mean) = diff(&a, &b);
            // Glyphs move by at most an eighth of a pixel; only edges change.
            assert!(max < 80, "max {max}");
            assert!(mean < 0.5, "mean {mean}");
            assert!(cache.masks.used_bytes() > 0);
            // Ink is there: the page is not just background.
            assert!(b.as_chunks::<4>().0.iter().any(|p| p[0] < 128));
        }
        // A repeat draw from the cache gives the same pixels.
        let mut cache = GlyphCache::new(1 << 20);
        let mut one = vec![0u8; (W * H * 4) as usize];
        let mut two = one.clone();
        let fonts = FontSet::bundled();
        render_page(&p, &fonts, None, &mut cache, &theme, W, H, &mut one).unwrap();
        render_page(&p, &fonts, None, &mut cache, &theme, W, H, &mut two).unwrap();
        assert_eq!(one, two);
    }

    #[test]
    fn large_text_and_edges() {
        // Above the mask limit: drawn from outlines, identical either way.
        let p = page("<p>Ag অ</p>", MASK_MAX_SIZE + 40.0);
        let (a, _) = draw(&p, false, &Theme::PAPER);
        let (b, cache) = draw(&p, true, &Theme::PAPER);
        assert_eq!(a, b);
        assert_eq!(cache.masks.used_bytes(), 0);
        // Glyphs hanging off every edge are clipped, not a panic.
        let mut p = page("<p>Edges এবং edges</p>", 30.0);
        for (i, line) in p.lines.iter_mut().enumerate() {
            for g in &mut line.glyphs {
                match i % 2 {
                    0 => {
                        g.x -= 40.0;
                        g.y -= 20.0;
                    }
                    _ => {
                        g.x += W as f32 - 60.0;
                        g.y += H as f32;
                    }
                }
            }
        }
        p.lines[0].glyphs[0].x = f32::NAN;
        let (_, _) = draw(&p, true, &Theme::NIGHT);
        let mut cache = GlyphCache::new(1 << 20);
        cache.clear();
        assert_eq!(cache.used_bytes(), 0);
    }
}
