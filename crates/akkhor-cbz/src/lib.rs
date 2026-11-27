//! Comic book archives (CBZ) for Akkhor: a zip of pictures, one per page.
//!
//! Pages are the archive's pictures in natural order (`p2` before `p10`).
//! `ComicInfo.xml`, when present, gives title, series, number, writer,
//! language, the front cover and right-to-left reading (manga). A page's
//! size comes from its picture's header, so opening and laying out cost
//! nothing; a page is decoded when drawn and kept until another page is
//! drawn, so at most one comic page sits in memory.

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
    /// The last page decoded.
    decoded: Mutex<Option<(usize, Arc<DecodedImage>)>>,
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
            decoded: Mutex::new(None),
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
        if let Ok(g) = self.decoded.lock()
            && let Some((i, img)) = g.as_ref()
            && *i == index
        {
            return Ok(img.clone());
        }
        let bytes = self.page_bytes(index)?;
        let mut img =
            decode_up_to(&bytes, MAX_PIXELS).ok_or(Error::malformed("undecodable picture"))?;
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
        let img = Arc::new(img);
        if let Ok(mut g) = self.decoded.lock() {
            *g = Some((index, img.clone()));
        }
        Ok(img)
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
        for oy in 0..h {
            let sy0 = f64::from(y + oy) * step;
            let sy1 = (sy0 + step).max(sy0 + 1.0);
            for ox in 0..w {
                let sx0 = f64::from(x + ox) * step;
                let sx1 = (sx0 + step).max(sx0 + 1.0);
                let px = average(&img, sx0, sy0, sx1, sy1);
                let a = u32::from(px[3]);
                let o = ((oy * w + ox) * 4) as usize;
                for c in 0..3 {
                    // Premultiplied, dimmed.
                    pixels[o + c] = ((u32::from(px[c]) * a / 255 * dim) >> 8) as u8;
                }
                pixels[o + 3] = px[3];
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

    /// Frees the decoded page.
    pub fn trim(&self) -> usize {
        self.decoded
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .map_or(0, |(_, img)| img.rgba.len())
    }
}

/// The average colour of the source area `[x0, x1) x [y0, y1)` (box
/// filter when shrinking, nearest pixel when enlarging).
fn average(img: &DecodedImage, x0: f64, y0: f64, x1: f64, y1: f64) -> [u8; 4] {
    let (w, h) = (img.width as usize, img.height as usize);
    let xa = (x0 as usize).min(w - 1);
    let ya = (y0 as usize).min(h - 1);
    let xb = (x1.ceil() as usize).clamp(xa + 1, w);
    let yb = (y1.ceil() as usize).clamp(ya + 1, h);
    // Large areas are sampled on a grid, so a zoomed-out page stays cheap.
    let sx = ((xb - xa) / 4).max(1);
    let sy = ((yb - ya) / 4).max(1);
    let mut acc = [0u32; 4];
    let mut n = 0u32;
    let mut yy = ya;
    while yy < yb {
        let row = yy * w;
        let mut xx = xa;
        while xx < xb {
            let p = (row + xx) * 4;
            for (a, &v) in acc.iter_mut().zip(&img.rgba[p..p + 4]) {
                *a += u32::from(v);
            }
            n += 1;
            xx += sx;
        }
        yy += sy;
    }
    acc.map(|v| (v / n.max(1)) as u8)
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
