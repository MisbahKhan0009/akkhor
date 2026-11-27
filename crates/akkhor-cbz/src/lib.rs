//! Comic book archives (CBZ) for Akkhor: a zip of pictures, one per page.
//!
//! Pages are the archive's pictures in natural order (`p2` before `p10`).
//! `ComicInfo.xml`, when present, gives title, series, number, writer,
//! language, the front cover and right-to-left reading (manga). A page's
//! size comes from its picture's header, so opening and laying out cost
//! nothing. A page is decoded when first drawn, once even when several
//! threads draw its tiles, and kept for redraws with the pages around it
//! up to 64 MB (always the last one used); `trim` frees them.

use std::sync::{Arc, Mutex, OnceLock};

use akkhor_core::zip::ZipArchive;
use akkhor_core::{Error, Result, Source, Theme, Tile};
use akkhor_flow::image::{DecodedImage, decode_up_to, image_size};
use akkhor_flow::markup::{Token, Tokenizer};

/// The largest page decoded at full size: 40 megapixels (a 5,000 x 8,000
/// scan). Bigger pictures are refused rather than risk running out of
/// memory.
const MAX_PIXELS: u64 = 40_000_000;
/// A decoded page is kept at most this big; larger ones are scaled down
/// once (to about 12 megapixels, still sharp at 3x zoom on a phone).
const KEEP_PIXELS: u64 = 12_000_000;

/// Decoded pages kept for redraws, the page being drawn ahead and a turn
/// back: at most this many bytes, and always the last one used.
const KEEP_BYTES: usize = 64 << 20;

/// A page being decoded or decoded: threads wanting the same page wait for
/// one decode instead of each doing it.
type Slot = Arc<OnceLock<Option<Arc<DecodedImage>>>>;

const PICTURES: [&str; 6] = [".jpg", ".jpeg", ".png", ".webp", ".gif", ".jfif"];

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub series: Option<String>,
    pub number: Option<String>,
    pub writers: Vec<String>,
    pub language: Option<String>,
    /// Pages read right to left (manga).
    pub right_to_left: bool,
    /// Page shown as the cover (0 when ComicInfo names none).
    pub cover_page: usize,
}

/// A drawn region of a page: premultiplied RGBA8, rows packed.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct CbzBook {
    zip: ZipArchive,
    /// Archive names of the pages, in reading order.
    pages: Vec<String>,
    pub metadata: Metadata,
    /// Pages decoded, most recently used last.
    decoded: Mutex<Vec<(usize, Slot)>>,
    /// Each page's size, read once from its picture's header.
    sizes: Vec<OnceLock<Option<(u32, u32)>>>,
}

impl CbzBook {
    pub fn open(source: Source) -> Result<Self> {
        let b = source.bytes();
        if b.starts_with(b"Rar!") {
            return Err(Error::malformed("a RAR archive (CBR), not supported"));
        }
        if b.starts_with(b"7z\xbc\xaf") {
            return Err(Error::malformed("a 7-Zip archive (CB7), not supported"));
        }
        let zip = ZipArchive::open(source)?;
        let mut pages: Vec<String> = zip
            .entries()
            .iter()
            .map(|e| e.name.clone())
            .filter(|n| is_page_name(n))
            .collect();
        pages.sort_by(|a, b| natural_cmp(a, b));
        if pages.is_empty() {
            return Err(Error::malformed("no pictures in the archive"));
        }
        let mut metadata = zip
            .entries()
            .iter()
            .find(|e| {
                e.name
                    .rsplit('/')
                    .next()
                    .unwrap_or(&e.name)
                    .eq_ignore_ascii_case("comicinfo.xml")
            })
            .and_then(|e| zip.read_string(&e.name).ok())
            .map(|xml| comic_info(&xml))
            .unwrap_or_default();
        metadata.cover_page = metadata.cover_page.min(pages.len() - 1);
        let sizes = pages.iter().map(|_| OnceLock::new()).collect();
        Ok(CbzBook {
            zip,
            pages,
            metadata,
            decoded: Mutex::new(Vec::new()),
            sizes,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Page size in pixels, from the picture's header.
    pub fn page_size(&self, index: usize) -> Result<(f32, f32)> {
        let slot = self.sizes.get(index).ok_or(Error::OutOfRange)?;
        let size = slot.get_or_init(|| {
            let bytes = self.page_bytes(index).ok()?;
            image_size(&bytes)
        });
        let (w, h) = size.ok_or(Error::malformed("unreadable picture"))?;
        Ok((w as f32, h as f32))
    }

    /// The picture's own bytes (for the cover and for sharing a page).
    pub fn page_bytes(&self, index: usize) -> Result<Vec<u8>> {
        self.zip
            .read(self.pages.get(index).ok_or(Error::OutOfRange)?)
    }

    pub fn cover_bytes(&self) -> Option<Vec<u8>> {
        self.page_bytes(self.metadata.cover_page).ok()
    }

    fn decode(&self, index: usize) -> Result<Arc<DecodedImage>> {
        if index >= self.pages.len() {
            return Err(Error::OutOfRange);
        }
        let slot = {
            let mut g = self.decoded.lock().unwrap_or_else(|e| e.into_inner());
            let slot = match g.iter().position(|(i, _)| *i == index) {
                Some(p) => g.remove(p).1,
                None => Slot::default(),
            };
            g.push((index, slot.clone()));
            slot
        };
        let img = slot.get_or_init(|| self.decode_page(index)).clone();
        self.keep_within_budget();
        img.ok_or(Error::malformed("undecodable picture"))
    }

    /// Drops the least recently used pages beyond [`KEEP_BYTES`].
    fn keep_within_budget(&self) {
        let mut g = self.decoded.lock().unwrap_or_else(|e| e.into_inner());
        let size = |s: &Slot| s.get().and_then(|o| o.as_ref()).map_or(0, |i| i.rgba.len());
        let mut total: usize = g.iter().map(|(_, s)| size(s)).sum();
        while total > KEEP_BYTES && g.len() > 1 {
            total -= size(&g.remove(0).1);
        }
    }

    fn decode_page(&self, index: usize) -> Option<Arc<DecodedImage>> {
        let bytes = self.page_bytes(index).ok()?;
        let mut img = decode_up_to(&bytes, MAX_PIXELS)?;
        let px = u64::from(img.width) * u64::from(img.height);
        if px > KEEP_PIXELS {
            let k = (KEEP_PIXELS as f64 / px as f64).sqrt();
            let (w, h) = (
                ((f64::from(img.width) * k) as u32).max(1),
                ((f64::from(img.height) * k) as u32).max(1),
            );
            img = DecodedImage {
                width: w,
                height: h,
                rgba: akkhor_flow::image::resize(&img, w, h),
            };
        }
        Some(Arc::new(img))
    }

    /// Draws `tile` (pixels of the page shown at `scale`; 1.0 = the
    /// picture's own pixels) of page `index`. Colours are kept; dark themes
    /// show pictures at 85% brightness, as for PDF photos.
    pub fn render_tile(
        &self,
        index: usize,
        scale: f32,
        tile: Tile,
        theme: &Theme,
    ) -> Result<Rendered> {
        let Tile {
            x,
            y,
            width: w,
            height: h,
        } = tile;
        let (pw, _) = self.page_size(index)?;
        let img = self.decode(index)?;
        // The decoded picture may be smaller than the page (kept smaller).
        let k = f64::from(img.width) / f64::from(pw.max(1.0));
        let step = k / f64::from(scale.max(1e-3));
        let dim = if theme.is_dark() { 217u32 } else { 256 };
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        // Each output pixel depends only on its place on the whole page, so
        // tiles join without seams. Columns and rows are worked out once.
        let iw = img.width as usize;
        let src = &img.rgba;
        let put = |o: usize, px: [u32; 4], pixels: &mut [u8]| {
            let a = px[3];
            let out = &mut pixels[o..o + 4];
            if a == 255 {
                // Nearly every comic page: no premultiplying to do.
                for c in 0..3 {
                    out[c] = ((px[c] * dim) >> 8) as u8;
                }
            } else {
                for c in 0..3 {
                    // Premultiplied, dimmed.
                    out[c] = ((px[c] * a / 255 * dim) >> 8) as u8;
                }
            }
            out[3] = a as u8;
        };
        if step < 1.0 {
            // Enlarged: bilinear, 8-bit weights.
            let axis = |from: u32, n: u32, len: u32| -> Vec<(usize, usize, u32)> {
                (0..n)
                    .map(|i| {
                        let c = ((f64::from(from + i) + 0.5) * step - 0.5).max(0.0);
                        let i0 = (c.floor() as usize).min(len as usize - 1);
                        let i1 = (i0 + 1).min(len as usize - 1);
                        (i0, i1, ((c - i0 as f64) * 256.0).round().min(256.0) as u32)
                    })
                    .collect()
            };
            let cols = axis(x, w, img.width);
            let rows = axis(y, h, img.height);
            // Each source row is blended across once, into packed pixels,
            // and kept while output rows still need it.
            let across = |row: usize, out: &mut Vec<u32>| {
                out.clear();
                let r = row * iw;
                out.extend(
                    cols.iter()
                        .map(|&(x0, x1, fx)| lerp(pack(src, r + x0), pack(src, r + x1), fx)),
                );
            };
            let (mut top, mut bottom) = (
                Vec::with_capacity(cols.len()),
                Vec::with_capacity(cols.len()),
            );
            let (mut have_top, mut have_bottom) = (usize::MAX, usize::MAX);
            for (oy, &(y0, y1, fy)) in rows.iter().enumerate() {
                if have_top != y0 {
                    if have_bottom == y0 {
                        std::mem::swap(&mut top, &mut bottom);
                        have_bottom = usize::MAX;
                    } else {
                        across(y0, &mut top);
                    }
                    have_top = y0;
                }
                if have_bottom != y1 {
                    across(y1, &mut bottom);
                    have_bottom = y1;
                }
                let out = &mut pixels[oy * w as usize * 4..(oy + 1) * w as usize * 4];
                for ((o, &t), &b) in out.as_chunks_mut::<4>().0.iter_mut().zip(&top).zip(&bottom) {
                    let v = lerp(t, b, fy).to_le_bytes();
                    if v[3] == 255 && dim == 256 {
                        *o = v;
                    } else {
                        let mut q = [0u8; 4];
                        put_into(&mut q, v.map(u32::from), dim);
                        *o = q;
                    }
                }
            }
        } else {
            // Shrunk: the average of the source pixels each one covers, on
            // a grid of at most 4 x 4 for large areas.
            let axis = |from: u32, n: u32, len: u32| -> Vec<(usize, usize, usize)> {
                (0..n)
                    .map(|i| {
                        let a0 = f64::from(from + i) * step;
                        let a = (a0 as usize).min(len as usize - 1);
                        let b = ((a0 + step).ceil() as usize).clamp(a + 1, len as usize);
                        (a, b, ((b - a) / 4).max(1))
                    })
                    .collect()
            };
            let cols = axis(x, w, img.width);
            let rows = axis(y, h, img.height);
            for (oy, &(ya, yb, sy)) in rows.iter().enumerate() {
                for (ox, &(xa, xb, sx)) in cols.iter().enumerate() {
                    let mut acc = [0u32; 4];
                    let mut n = 0u32;
                    for yy in (ya..yb).step_by(sy) {
                        let row = yy * iw;
                        for xx in (xa..xb).step_by(sx) {
                            let p = (row + xx) * 4;
                            for (a, &v) in acc.iter_mut().zip(&src[p..p + 4]) {
                                *a += u32::from(v);
                            }
                            n += 1;
                        }
                    }
                    let px = acc.map(|v| v / n.max(1));
                    put((oy * w as usize + ox) * 4, px, &mut pixels);
                }
            }
        }
        Ok(Rendered {
            width: w,
            height: h,
            pixels,
        })
    }

    /// The whole page at `scale`.
    pub fn render_page(&self, index: usize, scale: f32, theme: &Theme) -> Result<Rendered> {
        let (pw, ph) = self.page_size(index)?;
        let w = ((pw * scale).round() as u32).max(1);
        let h = ((ph * scale).round() as u32).max(1);
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(Error::malformed("page too large at this scale"));
        }
        self.render_tile(index, scale, Tile::new(0, 0, w, h), theme)
    }

    /// Frees the decoded pages; returns the bytes they held.
    pub fn trim(&self) -> usize {
        let mut g = self.decoded.lock().unwrap_or_else(|e| e.into_inner());
        g.drain(..)
            .filter_map(|(_, s)| s.get().cloned().flatten())
            .map(|img| img.rgba.len())
            .sum()
    }
}

/// The RGBA pixel at index `i`, packed little-endian.
#[inline]
fn pack(src: &[u8], i: usize) -> u32 {
    let p = &src[i * 4..i * 4 + 4];
    u32::from_le_bytes([p[0], p[1], p[2], p[3]])
}

/// Blends packed pixels `a` and `b`, `f`/256 of the way to `b`: red and
/// blue in one multiply, green and alpha in another.
#[inline]
fn lerp(a: u32, b: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = (((a & 0x00ff_00ff) * g + (b & 0x00ff_00ff) * f + 0x0080_0080) >> 8) & 0x00ff_00ff;
    let ga = ((((a >> 8) & 0x00ff_00ff) * g + ((b >> 8) & 0x00ff_00ff) * f + 0x0080_0080) >> 8)
        & 0x00ff_00ff;
    rb | (ga << 8)
}

/// Writes a straight-alpha pixel premultiplied and dimmed (`dim`/256).
#[inline]
fn put_into(out: &mut [u8; 4], px: [u32; 4], dim: u32) {
    let a = px[3];
    for c in 0..3 {
        out[c] = if a == 255 {
            ((px[c] * dim) >> 8) as u8
        } else {
            ((px[c] * a / 255 * dim) >> 8) as u8
        };
    }
    out[3] = a as u8;
}

fn is_page_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    !lower.ends_with('/')
        && !lower.starts_with("__macosx/")
        && !lower.contains("/__macosx/")
        && !file.starts_with('.')
        && PICTURES.iter().any(|ext| file.ends_with(ext))
}

/// Compares names as people sort them: runs of digits by value.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let da = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let db = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (na, nb) = (trim_zeros(&a[..da]), trim_zeros(&b[..db]));
                let ord = na.len().cmp(&nb.len()).then(na.cmp(nb)).then(da.cmp(&db));
                if ord.is_ne() {
                    return ord;
                }
                a = &a[da..];
                b = &b[db..];
            }
            (Some(x), Some(y)) => {
                let ord = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if ord.is_ne() {
                    return ord;
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let i = d
        .iter()
        .position(|&c| c != b'0')
        .unwrap_or(d.len().saturating_sub(1));
    &d[i..]
}

fn comic_info(xml: &str) -> Metadata {
    let mut m = Metadata::default();
    let mut current = String::new();
    let mut text = String::new();
    for tok in Tokenizer::new(xml) {
        match tok {
            Token::Start { name, attrs, .. } => {
                if name == "page" {
                    let kind = attrs
                        .iter()
                        .find(|(k, _)| k == "type")
                        .map(|(_, v)| v.as_str());
                    let image = attrs
                        .iter()
                        .find(|(k, _)| k == "image")
                        .and_then(|(_, v)| v.parse::<usize>().ok());
                    if kind.is_some_and(|k| k.eq_ignore_ascii_case("frontcover"))
                        && let Some(i) = image
                    {
                        m.cover_page = i;
                    }
                }
                current = name;
                text.clear();
            }
            Token::Text(t) => text.push_str(&t),
            Token::End { name } => {
                let t = text.trim().to_string();
                if name == current && !t.is_empty() {
                    match name.as_str() {
                        "title" => m.title = Some(t),
                        "series" => m.series = Some(t),
                        "number" => m.number = Some(t),
                        "writer" => {
                            m.writers = t
                                .split(',')
                                .map(|w| w.trim().to_string())
                                .filter(|w| !w.is_empty())
                                .collect();
                        }
                        "languageiso" => m.language = Some(t),
                        "manga" => m.right_to_left = t.eq_ignore_ascii_case("yesandrighttoleft"),
                        _ => {}
                    }
                }
                text.clear();
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec![
            "p10.jpg",
            "p2.jpg",
            "P1.jpg",
            "p02b.jpg",
            "a/p3.png",
            "cover.jpg",
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            [
                "a/p3.png",
                "cover.jpg",
                "P1.jpg",
                "p2.jpg",
                "p02b.jpg",
                "p10.jpg"
            ]
        );
    }

    #[test]
    fn page_names() {
        assert!(is_page_name("Ch1/001.JPG"));
        assert!(!is_page_name("__MACOSX/._001.jpg"));
        assert!(!is_page_name("Ch1/.hidden.png"));
        assert!(!is_page_name("ComicInfo.xml"));
        assert!(!is_page_name("Thumbs.db"));
    }
}
