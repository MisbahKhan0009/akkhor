//! Images inside reflowable books.
//!
//! Layout only needs an image's size, which is read from the first bytes of
//! the file (PNG, JPEG, GIF, WebP), so laying out a chapter never decodes a
//! picture. Pixels are decoded when a page is drawn, scaled to the size they
//! are shown at, and kept in a byte-budget cache.

/// Where a book's images come from. EPUB implements this over its zip.
pub trait ImageProvider {
    /// The raw bytes of the image at `src` (as written in the book).
    fn bytes(&self, src: &str) -> Option<Vec<u8>>;

    /// Pixel size of the image, read from its header.
    fn size(&self, src: &str) -> Option<(u32, u32)> {
        image_size(&self.bytes(src)?)
    }

    /// The image's pixels for drawing at `w` x `h` (the result may be any
    /// size; it is scaled). By default the bytes are decoded; a provider
    /// that draws its images (a PDF page region) renders them at that size.
    fn decoded(&self, src: &str, w: u32, h: u32) -> Option<DecodedImage> {
        let _ = (w, h);
        decode(&self.bytes(src)?)
    }
}

/// Decoded pixels: RGBA8, straight alpha, rows tightly packed.
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}
fn le24(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 3)?;
    Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
}

/// Width and height from an image header, without decoding.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(b, 16)?, be32(b, 20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((le16(b, 6)?, le16(b, 8)?));
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return match b.get(12..16)? {
            b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
            b"VP8L" => {
                let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(b, 24)? + 1, le24(b, 27)? + 1)),
            _ => None,
        };
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        // Walk JPEG segments to the start-of-frame marker.
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            if marker == 0xFF {
                i += 1;
                continue;
            }
            let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
            if is_sof {
                return Some((be16(b, i + 7)?, be16(b, i + 5)?));
            }
            if matches!(marker, 0xD8 | 0x01 | 0xD0..=0xD7) {
                i += 2;
                continue;
            }
            i += 2 + be16(b, i + 2)? as usize;
        }
    }
    None
}

/// Decodes PNG and JPEG (and WebP and GIF with `images-more`). Other
/// formats return `None` (the layout still
/// reserves their space and shows nothing).
#[cfg(feature = "images")]
pub fn decode(b: &[u8]) -> Option<DecodedImage> {
    if b.starts_with(b"\x89PNG") {
        return decode_png(b);
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        return decode_jpeg(b);
    }
    #[cfg(feature = "images-more")]
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return decode_webp(b);
    }
    #[cfg(feature = "images-more")]
    if b.starts_with(b"GIF8") {
        return decode_gif(b);
    }
    None
}

/// The largest picture decoded, in pixels: 16 million for book pictures
/// (64 MB while decoding, and it stops decompression bombs), more where a
/// caller (a comic page) asks for it.
#[cfg(feature = "images")]
pub fn decode_up_to(b: &[u8], max_pixels: u64) -> Option<DecodedImage> {
    let (w, h) = image_size(b)?;
    if u64::from(w) * u64::from(h) > max_pixels {
        return None;
    }
    if u64::from(w) * u64::from(h) <= MAX_PIXELS {
        return decode(b);
    }
    LIMIT.with(|l| {
        let old = l.replace(max_pixels);
        let out = decode(b);
        l.set(old);
        out
    })
}

#[cfg(feature = "images")]
thread_local! {
    static LIMIT: std::cell::Cell<u64> = const { std::cell::Cell::new(MAX_PIXELS) };
}

#[cfg(feature = "images")]
fn too_big(w: u32, h: u32) -> bool {
    u64::from(w) * u64::from(h) > LIMIT.with(std::cell::Cell::get)
}

#[cfg(feature = "images-more")]
fn decode_webp(b: &[u8]) -> Option<DecodedImage> {
    let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(b)).ok()?;
    let (w, h) = dec.dimensions();
    if too_big(w, h) {
        return None;
    }
    let mut buf = vec![0u8; dec.output_buffer_size()?];
    dec.read_image(&mut buf).ok()?;
    let rgba = if dec.has_alpha() {
        buf
    } else {
        let mut out = Vec::with_capacity(buf.len() / 3 * 4);
        for p in buf.as_chunks::<3>().0 {
            out.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        out
    };
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

#[cfg(feature = "images-more")]
fn decode_gif(b: &[u8]) -> Option<DecodedImage> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(std::io::Cursor::new(b)).ok()?;
    let (w, h) = (u32::from(dec.width()), u32::from(dec.height()));
    if too_big(w, h) {
        return None;
    }
    // The first frame, placed on a transparent canvas of the screen size.
    let frame = dec.read_next_frame().ok()??;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let (fl, ft) = (u32::from(frame.left), u32::from(frame.top));
    let (fw, fh) = (u32::from(frame.width), u32::from(frame.height));
    for y in 0..fh.min(h.saturating_sub(ft)) {
        for x in 0..fw.min(w.saturating_sub(fl)) {
            let s = ((y * fw + x) * 4) as usize;
            let d = (((y + ft) * w + x + fl) * 4) as usize;
            if let (Some(src), Some(dst)) = (frame.buffer.get(s..s + 4), rgba.get_mut(d..d + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    Some(DecodedImage {
        width: w,
        height: h,
        rgba,
    })
}

#[cfg(not(feature = "images"))]
pub fn decode(_b: &[u8]) -> Option<DecodedImage> {
    None
}

/// Refuse to decode images over 16 megapixels (64 MB while decoding):
/// book pictures are far smaller, and it stops decompression bombs.
#[cfg(feature = "images")]
const MAX_PIXELS: u64 = 16_000_000;

#[cfg(feature = "images")]
fn decode_png(b: &[u8]) -> Option<DecodedImage> {
    use png::{ColorType, Transformations};
    let (w, h) = image_size(b)?;
    if too_big(w, h) {
        return None;
    }
    let mut dec = png::Decoder::new(std::io::Cursor::new(b));
    dec.set_transformations(Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let px = &buf[..info.buffer_size()];
    let n = (info.width * info.height) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    match info.color_type {
        ColorType::Rgba => rgba.extend_from_slice(px),
        ColorType::Rgb => px
            .as_chunks::<3>()
            .0
            .iter()
            .for_each(|p| rgba.extend_from_slice(&[p[0], p[1], p[2], 255])),
        ColorType::GrayscaleAlpha => px
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|p| rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
        ColorType::Grayscale => px
            .iter()
            .for_each(|&g| rgba.extend_from_slice(&[g, g, g, 255])),
        ColorType::Indexed => return None, // expanded by normalize_to_color8
    }
    Some(DecodedImage {
        width: info.width,
        height: info.height,
        rgba,
    })
}

#[cfg(feature = "images")]
fn decode_jpeg(b: &[u8]) -> Option<DecodedImage> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let (w, h) = image_size(b)?;
    if too_big(w, h) {
        return None;
    }
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(b), opts);
    let rgba = dec.decode().ok()?;
    let info = dec.info()?;
    let (width, height) = (info.width as u32, info.height as u32);
    if rgba.len() != (width * height * 4) as usize {
        return None;
    }
    Some(DecodedImage {
        width,
        height,
        rgba,
    })
}

/// Scales straight-alpha RGBA to `w` x `h` by averaging (box filter), which
/// keeps downscaled photos and line art clean.
pub fn resize(img: &DecodedImage, w: u32, h: u32) -> Vec<u8> {
    let (sw, sh) = (img.width.max(1), img.height.max(1));
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let y0 = (y as u64 * sh as u64 / h as u64) as u32;
        let y1 = (((y + 1) as u64 * sh as u64).div_ceil(h as u64) as u32).clamp(y0 + 1, sh);
        for x in 0..w {
            let x0 = (x as u64 * sw as u64 / w as u64) as u32;
            let x1 = (((x + 1) as u64 * sw as u64).div_ceil(w as u64) as u32).clamp(x0 + 1, sw);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for sy in y0..y1 {
                let row = (sy * sw) as usize * 4;
                for sx in x0..x1 {
                    let p = row + sx as usize * 4;
                    for (a, &v) in acc.iter_mut().zip(&img.rgba[p..p + 4]) {
                        *a += v as u32;
                    }
                    n += 1;
                }
            }
            let o = ((y * w + x) * 4) as usize;
            for (dst, a) in out[o..o + 4].iter_mut().zip(acc) {
                *dst = (a / n.max(1)) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_header_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&300u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((300, 200)));

        let gif = b"GIF89a\x40\x01\xC8\x00";
        assert_eq!(image_size(gif), Some((320, 200)));

        // JPEG: SOI, an APP0 segment, then SOF0 with height 480, width 640.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01,
            0xE0, 0x02, 0x80, 0x03, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(image_size(&jpeg), Some((640, 480)));
        assert_eq!(image_size(b"not an image"), None);
    }

    #[test]
    fn resizes_by_averaging() {
        let img = DecodedImage {
            width: 2,
            height: 2,
            rgba: vec![
                0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
            ],
        };
        let out = resize(&img, 1, 1);
        assert_eq!(out, vec![127, 127, 127, 255]);
    }
}
