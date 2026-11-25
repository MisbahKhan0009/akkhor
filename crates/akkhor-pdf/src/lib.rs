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
pub mod recover;
pub mod reference;
pub mod text;

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
        let tables: Vec<Option<Arc<recover::Recovery>>> =
            glyphs.fonts.iter().map(|f| self.recovery(f)).collect();
        contain(|| {
            text::assemble(
                &glyphs,
                |word| {
                    let mut out = Vec::with_capacity(word.len());
                    let mut i = 0;
                    while i < word.len() {
                        let g = word[i];
                        let rec = tables.get(g.font).and_then(|t| t.as_deref());
                        // A run of glyphs that means more than its parts.
                        if let Some(r) = rec.filter(|r| r.max_sequence >= 2) {
                            let longest = r.max_sequence.min(word.len() - i);
                            let hit = (2..=longest).rev().find_map(|n| {
                                let run = &word[i..i + n];
                                if run.iter().any(|x| x.font != g.font) {
                                    return None;
                                }
                                let key: Vec<u32> = run.iter().map(|x| x.glyph_id).collect();
                                r.sequences.get(&key).map(|t| (n, t))
                            });
                            if let Some((n, t)) = hit {
                                let mut p =
                                    text::GlyphPiece::new(g, t.clone(), recover::Role::Base);
                                for x in &word[i + 1..i + n] {
                                    p.rect = p.rect.union(x.rect);
                                }
                                out.push(p);
                                i += n;
                                continue;
                            }
                        }
                        // An invisible, zero-width glyph in a recovered font
                        // is a shaping leftover, whatever the text layer says.
                        if g.advance == 0.0
                            && rec.is_some_and(|r| r.indic && r.blank.contains(&g.glyph_id))
                        {
                            i += 1;
                            continue;
                        }
                        let recovered = rec.and_then(|r| r.map.get(&g.glyph_id));
                        let own = g.unicode.as_deref().filter(|u| text::usable(u));
                        let (t, role) = match (recovered, own) {
                            (Some(r), _) if rec.is_some_and(|r| r.indic) => {
                                (r.text.clone(), r.role)
                            }
                            (_, Some(t)) => (recover::canonical(t), recover::role_of(t)),
                            (Some(r), None) => (r.text.clone(), r.role),
                            (None, None) => (String::new(), recover::Role::Base),
                        };
                        out.push(text::GlyphPiece::new(g, t, role));
                        i += 1;
                    }
                    out
                },
                text::reorder,
            )
        })
    }

    /// The recovery table for a font, built once per document.
    fn recovery(&self, font: &PdfFont) -> Option<Arc<recover::Recovery>> {
        let data = font.data.as_ref()?;
        let mut cache = self.recoveries.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .entry(font.key)
            .or_insert_with(|| {
                contain(|| reference::recover(data))
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
