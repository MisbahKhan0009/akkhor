//! Fonts, one per script, loaded lazily.
//!
//! A reader that ships every Indic font would be heavy. Here each font is
//! only a registration until a book actually contains its script; then the
//! file is memory-mapped (not read) and parsed. An English-only book never
//! touches the Bangla or Devanagari font at all.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use akkhor_core::Source;
use harfrust::font::{Blob, Font};

use crate::script::Script;

/// Index of a font inside a [`FontSet`].
pub type FontId = usize;

pub enum FontData {
    Static(&'static [u8]),
    Path(PathBuf),
    Source(Source),
}

/// A parsed font plus the metrics layout needs, in font units.
pub struct LoadedFont {
    pub font: Font,
    pub units_per_em: f32,
    pub ascender: f32,
    pub descender: f32,
}

impl LoadedFont {
    fn load(data: &FontData) -> Option<Self> {
        let blob: Blob = match data {
            FontData::Static(b) => Blob::from(*b),
            FontData::Path(p) => {
                let src = Source::open(p).ok()?;
                let shared: Arc<dyn AsRef<[u8]> + Send + Sync> = src.shared();
                Blob::from(shared)
            }
            FontData::Source(src) => {
                let shared: Arc<dyn AsRef<[u8]> + Send + Sync> = src.shared();
                Blob::from(shared)
            }
        };
        let font = Font::new(blob, 0)?;
        let metrics = font.metrics();
        let units_per_em = metrics.units_per_em.max(1) as f32;
        let (ascender, descender) = metrics
            .hhea_line
            .as_ref()
            .map(|l| (l.ascender.to_f32(), l.descender.to_f32()))
            .unwrap_or((units_per_em * 0.9, -units_per_em * 0.3));
        Some(Self {
            font,
            units_per_em,
            ascender,
            descender,
        })
    }

    pub fn has_char(&self, c: char) -> bool {
        self.font.charmap().map_unicode(c as u32).is_some()
    }
}

struct Entry {
    scripts: Vec<Script>,
    data: FontData,
    loaded: OnceLock<Option<LoadedFont>>,
}

/// The fonts a document is laid out with.
#[derive(Default)]
pub struct FontSet {
    entries: Vec<Entry>,
}

impl FontSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a font for some scripts. The first font registered for
    /// `Script::Latin` also serves as the fallback for everything else.
    /// Nothing is read until the font is first needed.
    pub fn add(&mut self, scripts: &[Script], data: FontData) -> FontId {
        self.entries.push(Entry {
            scripts: scripts.to_vec(),
            data,
            loaded: OnceLock::new(),
        });
        self.entries.len() - 1
    }

    /// Noto Serif, Noto Serif Bengali and Noto Serif Devanagari, compiled in.
    #[cfg(feature = "bundled-fonts")]
    pub fn bundled() -> Self {
        let mut set = Self::new();
        set.add(
            &[Script::Latin, Script::Common, Script::Other],
            FontData::Static(include_bytes!("../../../fonts/NotoSerif-Regular.ttf")),
        );
        set.add(
            &[Script::Bengali],
            FontData::Static(include_bytes!(
                "../../../fonts/NotoSerifBengali-Regular.ttf"
            )),
        );
        set.add(
            &[Script::Devanagari],
            FontData::Static(include_bytes!(
                "../../../fonts/NotoSerifDevanagari-Regular.ttf"
            )),
        );
        set
    }

    pub fn get(&self, id: FontId) -> Option<&LoadedFont> {
        let e = self.entries.get(id)?;
        e.loaded.get_or_init(|| LoadedFont::load(&e.data)).as_ref()
    }

    /// The font for `script`, falling back to the Latin font.
    pub fn for_script(&self, script: Script) -> Option<(FontId, &LoadedFont)> {
        let pick = |s: Script| {
            self.entries
                .iter()
                .position(|e| e.scripts.contains(&s))
                .and_then(|id| self.get(id).map(|f| (id, f)))
        };
        pick(script)
            .or_else(|| pick(Script::Latin))
            .or_else(|| (0..self.entries.len()).find_map(|id| self.get(id).map(|f| (id, f))))
    }

    /// How many fonts have actually been loaded so far.
    pub fn loaded_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.loaded.get().is_some())
            .count()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;

    #[test]
    fn fonts_load_lazily() {
        let set = FontSet::bundled();
        assert_eq!(set.loaded_count(), 0);
        let (id, f) = set.for_script(Script::Bengali).unwrap();
        assert_eq!(id, 1);
        assert!(f.has_char('ক'));
        assert_eq!(set.loaded_count(), 1);
    }
}
