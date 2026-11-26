//! Reading themes on PDF pages.
//!
//! A PDF paints dark text on what it assumes is a white page. Painting a
//! dark background under it hides the text, and inverting the whole page
//! turns photographs into negatives. Instead the page is drawn on white and
//! then every pixel outside a colour image is remapped by lightness: white
//! becomes the theme's background, black its text colour, and colours keep
//! their hue. Colour images keep their real colours. Greyscale and 1-bit
//! images (scanned pages) are remapped like text, so a scanned book reads
//! in the theme too.

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, Paint, SoftMask, interpret_page,
};
use hayro::hayro_syntax::object::{Array, Name, Object};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{BezPath, Rect};

use akkhor_core::{Theme, Tile};

/// Where a page's colour images land, in points from the top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageBoxes(pub Vec<Rect>);

struct Collector {
    boxes: Vec<Rect>,
    /// Greyscale and 1-bit pictures too (for reflow), not only colour ones,
    /// and the vector shapes page structure looks at.
    all: bool,
    rules: Vec<Rect>,
    fills: Vec<Rect>,
}

/// The vector shapes of a page that page structure uses: thin lines
/// (table rules, frames) and filled rectangles (shaded boxes, cells), in
/// points from the top left. Glyphs are not paths here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorShapes {
    pub rules: Vec<Rect>,
    pub fills: Vec<Rect>,
}

/// Thinner than this (points) is a line, not an area.
const RULE: f64 = 2.5;

impl Collector {
    fn rule(&mut self, r: Rect) {
        let long = r.width().max(r.height());
        if long >= 4.0 && r.width().min(r.height()) <= RULE {
            self.rules.push(r);
        }
    }
}

/// The colour as painted over white, and whether it shows there.
pub(crate) fn shows_on_white(paint: &Paint<'_>) -> bool {
    match paint {
        Paint::Color(c) => {
            let [r, g, b, a] = c.to_rgba().premultiplied();
            let over = |ch: f32| ch + (1.0 - a);
            a > 0.05 && [r, g, b].into_iter().map(over).any(|ch| ch < 0.985)
        }
        // Shadings and patterns: drawn, but not a plain rule or box.
        Paint::Pattern(_) => false,
    }
}

impl<'a> Device<'a> for Collector {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, mode: &DrawMode) {
        if !self.all || !shows_on_white(&props.paint) {
            return;
        }
        let path = props.transform * path.clone();
        let (fill, stroke) = match mode {
            DrawMode::Fill(_) => (true, None),
            DrawMode::Stroke(s) => (false, Some(s)),
            DrawMode::FillAndStroke(_, s) => (true, Some(s)),
            DrawMode::Invisible => return,
        };
        if fill {
            use hayro::kurbo::Shape;
            let bb = path.bounding_box();
            if bb.width().min(bb.height()) <= RULE {
                self.rule(bb);
            } else if bb.width() >= 12.0
                && bb.height() >= 6.0
                && path.area().abs() >= bb.area() * 0.85
            {
                // A rectangle, rounded corners allowed: not a glyph-like
                // shape, a chart or a curve.
                self.fills.push(bb);
            }
        }
        if let Some(s) = stroke {
            let scale = props.transform.determinant().abs().sqrt();
            let half = (f64::from(s.line_width) * scale).max(0.3) / 2.0;
            for seg in path.segments() {
                if let hayro::kurbo::PathSeg::Line(l) = seg {
                    let (a, b) = (l.p0, l.p1);
                    if (a.y - b.y).abs() < 0.5 {
                        self.rule(Rect::new(
                            a.x.min(b.x),
                            a.y - half,
                            a.x.max(b.x),
                            a.y + half,
                        ));
                    } else if (a.x - b.x).abs() < 0.5 {
                        self.rule(Rect::new(
                            a.x - half,
                            a.y.min(b.y),
                            a.x + half,
                            a.y.max(b.y),
                        ));
                    }
                }
            }
        }
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        // Stencil masks are painted in the fill colour: like text.
        let Image::Raster(raster) = image else { return };
        if !self.all && is_greyscale(raster.stream()) {
            return;
        }
        // The transform maps image pixels, not the unit square.
        let (w, h) = (raster.width() as f64, raster.height() as f64);
        let r = props
            .transform
            .transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
        if r.width() > 0.5 && r.height() > 0.5 {
            self.boxes.push(r);
        }
    }
}

/// Greyscale or 1-bit: a scan of text, not a picture.
fn is_greyscale(stream: &hayro::hayro_syntax::object::Stream<'_>) -> bool {
    let dict = stream.dict();
    let fax = |n: &str| matches!(n, "JBIG2Decode" | "CCITTFaxDecode" | "CCF");
    if dict
        .get::<Name<'_>>(b"Filter")
        .is_some_and(|n| fax(n.as_str()))
        || dict
            .get::<Array<'_>>(b"Filter")
            .is_some_and(|a| a.iter::<Name<'_>>().any(|n| fax(n.as_str())))
    {
        return true;
    }
    if dict.get::<i32>(b"BitsPerComponent") == Some(1)
        || dict.get::<bool>(b"ImageMask") == Some(true)
    {
        return true;
    }
    if let Some(name) = dict.get::<Name<'_>>(b"ColorSpace") {
        return matches!(name.as_str(), "DeviceGray" | "G" | "CalGray");
    }
    if let Some(array) = dict.get::<Array<'_>>(b"ColorSpace") {
        let mut items = array.iter::<Object<'_>>();
        let kind = items.next().and_then(|o| o.into_name());
        return match kind.as_ref().map(|n| n.as_str()) {
            Some("CalGray") => true,
            // An ICC profile with one component.
            Some("ICCBased") => {
                items
                    .next()
                    .and_then(|o| o.into_stream())
                    .and_then(|s| s.dict().get::<i32>(b"N"))
                    == Some(1)
            }
            _ => false,
        };
    }
    false
}

/// The colour images on `page` (a light pass: nothing is decoded or drawn).
pub(crate) fn image_boxes(page: &Page<'_>) -> ImageBoxes {
    boxes(page, false)
}

/// All pictures of a page (greyscale ones included; stencil masks paint
/// like text) and its vector rules and filled boxes, for reflow, in one
/// pass, in reading orientation (see [`crate::glyphs::collect`]).
pub(crate) fn page_shapes(page: &Page<'_>) -> (Vec<Rect>, VectorShapes) {
    let c = collect(page, true, true);
    (
        c.boxes,
        VectorShapes {
            rules: c.rules,
            fills: c.fills,
        },
    )
}

fn boxes(page: &Page<'_>, all: bool) -> ImageBoxes {
    ImageBoxes(collect(page, all, false).boxes)
}

fn collect(page: &Page<'_>, all: bool, upright: bool) -> Collector {
    let (transform, (w, h)) = crate::glyphs::page_transform(page, upright);
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        transform,
        Rect::new(0.0, 0.0, w as f64, h as f64),
        &cache,
        page.xref(),
        crate::interpreter_settings(),
    );
    let mut collector = Collector {
        boxes: Vec::new(),
        all,
        rules: Vec::new(),
        fills: Vec::new(),
    };
    interpret_page(page, &mut ctx, &mut collector);
    collector
}

/// Whether a theme changes the page at all: black on white is the PDF as
/// designed.
pub(crate) fn is_original(theme: &Theme) -> bool {
    let (b, t) = (theme.background, theme.text);
    (b.r, b.g, b.b) == (255, 255, 255) && (t.r, t.g, t.b) == (0, 0, 0)
}

/// Remaps a rendered tile (opaque RGBA8, drawn on white) to `theme`,
/// leaving pixels inside colour images as they are.
pub(crate) fn apply(out: &mut [u8], tile: Tile, scale: f32, boxes: &ImageBoxes, theme: &Theme) {
    if is_original(theme) {
        return;
    }
    let (w, h) = (tile.width as usize, tile.height as usize);
    // Image boxes in tile pixels, as [x0, x1) × [y0, y1).
    let s = scale as f64;
    let images: Vec<(usize, usize, usize, usize)> = boxes
        .0
        .iter()
        .filter_map(|r| {
            let x0 = (r.x0 * s - tile.x as f64).max(0.0).ceil() as usize;
            let y0 = (r.y0 * s - tile.y as f64).max(0.0).ceil() as usize;
            let x1 = ((r.x1 * s - tile.x as f64).floor().max(0.0) as usize).min(w);
            let y1 = ((r.y1 * s - tile.y as f64).floor().max(0.0) as usize).min(h);
            (x0 < x1 && y0 < y1).then_some((x0, x1, y0, y1))
        })
        .collect();
    let bg = [theme.background.r, theme.background.g, theme.background.b];
    let fg = [theme.text.r, theme.text.g, theme.text.b].map(i32::from);
    // The theme colour for each lightness, computed once.
    let mut base = [[0i32; 3]; 256];
    for (luma, b) in base.iter_mut().enumerate() {
        let l = luma as i32;
        for c in 0..3 {
            b[c] = (fg[c] * (255 - l) + i32::from(bg[c]) * l + 127) / 255;
        }
    }
    // Colour is kept a little softer on dark themes, where full saturation
    // glares.
    let chroma = if theme.is_dark() { 200 } else { 256 };
    // Rows are swept top to bottom with the boxes that cover them; each row
    // gets a coverage count per pixel from a difference array, so the cost
    // is the tile's pixels plus the boxes' heights, however many images a
    // page draws (a page tiled with 100,000 small images stays linear).
    let mut images = images;
    images.sort_unstable_by_key(|b| b.2);
    let mut next = 0;
    let mut active: Vec<(usize, usize, usize)> = Vec::new();
    let mut cover = vec![0i32; w + 1];
    for (y, row) in out.chunks_exact_mut(w * 4).take(h).enumerate() {
        active.retain(|b| y < b.2);
        while next < images.len() && images[next].2 <= y {
            let (x0, x1, _, y1) = images[next];
            active.push((x0, x1, y1));
            next += 1;
        }
        let covered = !active.is_empty();
        if covered {
            cover.fill(0);
            for &(x0, x1, _) in &active {
                cover[x0] += 1;
                cover[x1] -= 1;
            }
            let mut c = 0;
            for v in cover.iter_mut() {
                c += *v;
                *v = c;
            }
        }
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if covered && cover[x] > 0 {
                continue;
            }
            // Most of a page is blank paper.
            if px[..3] == [255, 255, 255] {
                *px = [bg[0], bg[1], bg[2], 255];
                continue;
            }
            let [r, g, b] = [px[0], px[1], px[2]].map(i32::from);
            let luma = ((r * 77 + g * 151 + b * 28) >> 8) as usize;
            let t = base[luma];
            let l = luma as i32;
            *px = [
                (t[0] + (r - l) * chroma / 256).clamp(0, 255) as u8,
                (t[1] + (g - l) * chroma / 256).clamp(0, 255) as u8,
                (t[2] + (b - l) * chroma / 256).clamp(0, 255) as u8,
                255,
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgb: [u8; 3], theme: &Theme) -> [u8; 3] {
        let mut px = [rgb[0], rgb[1], rgb[2], 255];
        apply(
            &mut px,
            Tile::new(0, 0, 1, 1),
            1.0,
            &ImageBoxes::default(),
            theme,
        );
        [px[0], px[1], px[2]]
    }

    #[test]
    fn white_and_black_become_the_theme() {
        let t = Theme::NIGHT;
        assert_eq!(
            pixel([255, 255, 255], &t),
            [t.background.r, t.background.g, t.background.b]
        );
        assert_eq!(pixel([0, 0, 0], &t), [t.text.r, t.text.g, t.text.b]);
    }

    #[test]
    fn colours_keep_their_hue() {
        let [r, g, b] = pixel([200, 30, 30], &Theme::NIGHT);
        assert!(r > g + 40 && r > b + 40, "red stays red: {r} {g} {b}");
    }

    #[test]
    fn image_pixels_are_left_alone() {
        let mut px = [10u8, 200, 30, 255].repeat(4); // 2×2 tile
        let boxes = ImageBoxes(vec![Rect::new(0.0, 0.0, 1.0, 2.0)]);
        apply(&mut px, Tile::new(0, 0, 2, 2), 1.0, &boxes, &Theme::NIGHT);
        assert_eq!(&px[0..4], &[10, 200, 30, 255]); // inside the image
        assert_ne!(&px[4..8], &[10, 200, 30, 255]); // outside
    }

    #[test]
    fn many_overlapping_images_are_kept_exactly() {
        // Pseudo-random boxes, many overlapping; every pixel inside any box
        // must keep its colour, every other pixel must be themed.
        let (w, h) = (64u32, 48u32);
        let mut seed = 7u64;
        let mut rnd = |m: f64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64 * m
        };
        let boxes: Vec<Rect> = (0..300)
            .map(|_| {
                let (x, y) = (rnd(70.0) - 5.0, rnd(55.0) - 5.0);
                Rect::new(x, y, x + rnd(12.0), y + rnd(9.0))
            })
            .collect();
        let tile = Tile::new(3, 2, w, h);
        let colour = [10u8, 200, 90, 255];
        let mut out: Vec<u8> = colour.repeat((w * h) as usize);
        apply(
            &mut out,
            tile,
            1.0,
            &ImageBoxes(boxes.clone()),
            &Theme::NIGHT,
        );
        for y in 0..h as usize {
            for x in 0..w as usize {
                let inside = boxes.iter().any(|r| {
                    let x0 = (r.x0 - 3.0).max(0.0).ceil() as usize;
                    let y0 = (r.y0 - 2.0).max(0.0).ceil() as usize;
                    let x1 = (r.x1 - 3.0).floor().max(0.0) as usize;
                    let y1 = (r.y1 - 2.0).floor().max(0.0) as usize;
                    x0 <= x && x < x1 && y0 <= y && y < y1
                });
                let px = &out[(y * w as usize + x) * 4..][..4];
                assert_eq!(px == colour, inside, "pixel {x},{y}");
            }
        }
    }
}
