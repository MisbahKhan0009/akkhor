//! Reference fonts: full copies of fonts that PDFs embed as stripped subsets.
//!
//! PDF producers subset fonts and usually drop the shaping tables (`GSUB`)
//! and the character map on the way; LibreOffice also renumbers the glyphs.
//! Reverse shaping then has nothing to run. But a glyph's outline survives
//! subsetting unchanged, so each embedded glyph can be found in a complete
//! copy of the same font by its outline, and the complete copy's shaping
//! rules then say which letters the glyph stands for.
//!
//! Reference fonts come from the app (the phone's own Noto fonts, through
//! [`register_reference_font`]) and, with the `bundled-fonts` feature, from
//! the engine's bundled fonts. Each one is only parsed the first time a PDF
//! needs it.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::recover::{self, GlyphText, Recovery};
use crate::shape::Shape;
use crate::stored;

type Data = Arc<dyn AsRef<[u8]> + Send + Sync>;

static GLYPH_NAMES: AtomicBool = AtomicBool::new(true);

/// Whether glyph names may help recovery (on by default). For tests: tables
/// already built keep what they learned.
#[doc(hidden)]
pub fn set_glyph_names(enabled: bool) {
    GLYPH_NAMES.store(enabled, Ordering::Relaxed);
}

fn build(data: &Data, deep: bool) -> Option<Recovery> {
    recover::build_with(data, GLYPH_NAMES.load(Ordering::Relaxed), deep)
}

struct ReferenceFont {
    data: Data,
    /// Glyph letters, from reverse shaping the complete font: the quick
    /// build, and the deep one for PDFs that use rarer clusters.
    recovery: OnceLock<Option<Arc<Recovery>>>,
    deep: OnceLock<Option<Arc<Recovery>>>,
    /// Outline hash → glyph id.
    shapes: OnceLock<HashMap<u64, u32>>,
    /// Coarse pictures of every glyph, for Type 3 matching: (glyph id,
    /// advance in 1000-unit ems, shape), 264 bytes a glyph.
    pictures: OnceLock<Vec<(u32, f32, Shape)>>,
    name: OnceLock<Option<String>>,
    /// Hash of the font's bytes, naming its tables on disk.
    key: OnceLock<u64>,
}

/// A font name without subset prefix, spaces, dashes or case:
/// "ABCDEF+Noto Serif-Bengali" and "NotoSerifBengali" are the same.
fn normalized_name(name: &str) -> String {
    let name = match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 => rest,
        _ => name,
    };
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl ReferenceFont {
    fn key(&self) -> u64 {
        *self
            .key
            .get_or_init(|| stored::font_key((*self.data).as_ref()))
    }

    /// A recovery table, from disk when kept there (see `stored`).
    fn table(&self, deep: bool) -> Option<Arc<Recovery>> {
        let names = GLYPH_NAMES.load(Ordering::Relaxed);
        let kind = format!("{}{}", if deep { "deep" } else { "quick" }, u8::from(names));
        stored::cached(
            self.key(),
            &kind,
            || build(&self.data, deep),
            stored::write_recovery,
            stored::read_recovery,
        )
        .map(Arc::new)
    }

    fn recovery(&self) -> Option<&Arc<Recovery>> {
        self.recovery.get_or_init(|| self.table(false)).as_ref()
    }

    fn deep_recovery(&self) -> Option<&Arc<Recovery>> {
        self.deep.get_or_init(|| self.table(true)).as_ref()
    }

    /// The PostScript name, normalized (see [`normalized_name`]).
    fn name(&self) -> &Option<String> {
        self.name.get_or_init(|| {
            let font = FontRef::new((*self.data).as_ref()).ok()?;
            let ps = font
                .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
                .english_or_first()?
                .to_string();
            Some(normalized_name(&ps))
        })
    }

    fn shapes(&self) -> &HashMap<u64, u32> {
        self.shapes.get_or_init(|| {
            stored::cached(
                self.key(),
                "shapes",
                || {
                    let mut index = HashMap::new();
                    for (gid, hash) in outline_hashes((*self.data).as_ref()).0 {
                        index.entry(hash).or_insert(gid);
                    }
                    index
                },
                stored::write_shapes,
                stored::read_shapes,
            )
        })
    }
}

impl ReferenceFont {
    fn pictures(&self) -> &[(u32, f32, Shape)] {
        self.pictures.get_or_init(|| {
            stored::cached(
                self.key(),
                "pictures",
                || self.draw_pictures(),
                |v, w| stored::write_pictures(v, w),
                stored::read_pictures,
            )
        })
    }

    fn draw_pictures(&self) -> Vec<(u32, f32, Shape)> {
        {
            let mut out = Vec::new();
            let Ok(font) = FontRef::new((*self.data).as_ref()) else {
                return out;
            };
            let upem = font.head().map(|h| h.units_per_em()).unwrap_or(1000).max(1) as f64;
            let scale = 1000.0 / upem;
            let count = font.maxp().map(|m| m.num_glyphs() as u32).unwrap_or(0);
            let outlines = font.outline_glyphs();
            let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
            for gid in 1..count {
                let id = GlyphId::new(gid);
                let Some(glyph) = outlines.get(id) else {
                    continue;
                };
                let mut pen = PathPen {
                    path: hayro::kurbo::BezPath::new(),
                    scale,
                };
                let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
                if glyph.draw(settings, &mut pen).is_err() || pen.path.elements().is_empty() {
                    continue;
                }
                let shape = Shape::of(&pen.path);
                if shape.is_empty() {
                    continue;
                }
                let advance = metrics.advance_width(id).unwrap_or(0.0) * scale as f32;
                out.push((gid, advance, shape));
            }
            out
        }
    }
}

struct PathPen {
    path: hayro::kurbo::BezPath,
    scale: f64,
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path
            .move_to((x as f64 * self.scale, y as f64 * self.scale));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path
            .line_to((x as f64 * self.scale, y as f64 * self.scale));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let s = self.scale;
        self.path
            .quad_to((cx as f64 * s, cy as f64 * s), (x as f64 * s, y as f64 * s));
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let s = self.scale;
        self.path.curve_to(
            (c0x as f64 * s, c0y as f64 * s),
            (c1x as f64 * s, c1y as f64 * s),
            (x as f64 * s, y as f64 * s),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// The letters of glyphs known only by their look (Type 3 synthetic bold):
/// each shape is compared with every registered font's glyphs of about the
/// same advance (synthetic bold keeps it) as a bolder copy of them, and the
/// best match is taken when it is near certain and clearly ahead of the
/// next glyph. Shapes are `(shape, advance)` in 1000-unit ems.
///
/// Each answer also says whether the shape is drawn bolder than the glyph
/// it matched (more ink): Chrome draws synthetic bold this way, and the
/// PDF says nothing else about it.
pub fn recognise(shapes: &[(Shape, f32)]) -> Vec<Option<Recognised>> {
    // A book draws the same bold glyphs on page after page: answers are
    // kept (up to a few thousand shapes) so each is matched once.
    type Seen = HashMap<(Shape, u32), Option<Recognised>>;
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    let key = |(s, a): &(Shape, f32)| (*s, a.round() as u32);
    let known: Vec<Option<Option<Recognised>>> = {
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        shapes.iter().map(|k| seen.get(&key(k)).cloned()).collect()
    };
    let todo: Vec<(Shape, f32)> = shapes
        .iter()
        .zip(&known)
        .map(|(s, k)| {
            if k.is_some() {
                (Shape::default(), s.1)
            } else {
                *s
            }
        })
        .collect();
    let fresh = match_shapes(&todo);
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() > 4096 {
        seen.clear();
    }
    shapes
        .iter()
        .zip(known)
        .zip(fresh)
        .map(|((s, k), f)| match k {
            Some(k) => k,
            None => {
                if !s.0.is_empty() {
                    seen.insert(key(s), f.clone());
                }
                f
            }
        })
        .collect()
}

/// Reference fonts pictured for shape matching: the first ones registered
/// (the app registers its script fonts first). Picturing a font costs a few
/// tens of milliseconds, once.
const MAX_PICTURED: usize = 24;

/// A Type 3 glyph read by its shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Recognised {
    pub text: GlyphText,
    /// Drawn with more ink than the font's own glyph: synthetic bold.
    pub bolder: bool,
}

fn match_shapes(shapes: &[(Shape, f32)]) -> Vec<Option<Recognised>> {
    const MIN: f32 = 0.9;
    const MARGIN: f32 = 0.03;
    // Per shape: (score, text) of the best glyph, and the best score of a
    // glyph that reads differently (fonts hold look-alike variants of the
    // same letters, which must not count against each other).
    let mut best: Vec<Option<(f32, GlyphText, u32)>> = vec![None; shapes.len()];
    let mut other: Vec<f32> = vec![0.0; shapes.len()];
    for font in references().into_iter().take(MAX_PICTURED) {
        let pictures = font.pictures();
        if pictures.is_empty() {
            continue;
        }
        // The quick table: the deep one costs too much for a heading, and
        // the glyphs of headings are common ones.
        let Some(table) = font.recovery().cloned() else {
            continue;
        };
        for (i, (shape, advance)) in shapes.iter().enumerate() {
            if shape.is_empty() {
                continue;
            }
            for (gid, adv, pic) in pictures {
                if (adv - advance).abs() > advance.max(100.0) * 0.12 {
                    continue;
                }
                let Some(text) = table.map.get(gid) else {
                    continue;
                };
                let s = pic.bold_match(shape, &pic.dilated());
                match &best[i] {
                    Some((f, t, _)) if s <= *f => {
                        if t.text != text.text {
                            other[i] = other[i].max(s);
                        }
                    }
                    _ => {
                        if let Some((f, t, _)) = &best[i]
                            && t.text != text.text
                        {
                            other[i] = other[i].max(*f);
                        }
                        best[i] = Some((s, text.clone(), pic.ink()));
                    }
                }
            }
        }
    }
    best.into_iter()
        .zip(other)
        .zip(shapes)
        .map(|((b, second), (shape, _))| {
            let (score, text, ink) = b?;
            (score >= MIN && score - second >= MARGIN).then(|| Recognised {
                text,
                // Emboldening grows every stroke by a fraction of the em:
                // a few percent more ink at this resolution.
                bolder: shape.ink() * 100 > ink * 106,
            })
        })
        .collect()
}

fn registry() -> &'static Mutex<Vec<Arc<ReferenceFont>>> {
    static REGISTRY: OnceLock<Mutex<Vec<Arc<ReferenceFont>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        #[allow(unused_mut)]
        let mut fonts: Vec<Arc<ReferenceFont>> = Vec::new();
        #[cfg(feature = "bundled-fonts")]
        for (_, bytes) in akkhor_flow::fonts::BUNDLED {
            let data: Data = Arc::new(*bytes);
            fonts.push(Arc::new(ReferenceFont {
                data,
                recovery: OnceLock::new(),
                deep: OnceLock::new(),
                shapes: OnceLock::new(),
                pictures: OnceLock::new(),
                name: OnceLock::new(),
                key: OnceLock::new(),
            }));
        }
        Mutex::new(fonts)
    })
}

/// Adds a complete font that PDFs may embed in stripped form (for example
/// `/system/fonts/NotoSansBengali-Regular.ttf`). Cheap: the font is only
/// read when a PDF needs it.
pub fn register_reference_font(data: Arc<dyn AsRef<[u8]> + Send + Sync>) {
    let font = Arc::new(ReferenceFont {
        data,
        recovery: OnceLock::new(),
        deep: OnceLock::new(),
        shapes: OnceLock::new(),
        pictures: OnceLock::new(),
        name: OnceLock::new(),
        key: OnceLock::new(),
    });
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(font);
}

fn references() -> Vec<Arc<ReferenceFont>> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Whether a font program can be shaped on its own.
fn has_shaping_rules(data: &[u8]) -> bool {
    FontRef::new(data).is_ok_and(|f| f.gsub().is_ok())
}

/// The recovery table for an embedded font: its own (cmap and shaping
/// rules) when it has shaping rules, otherwise borrowed glyph by glyph from
/// the reference font whose outlines it matches best.
pub fn recover(data: &Data, name: Option<&str>) -> Option<Recovery> {
    let own = build(data, false);
    let bytes = (**data).as_ref();
    if has_shaping_rules(bytes) {
        return own.map(|mut r| {
            r.complete = true;
            r
        });
    }
    let (embedded, blank) = outline_hashes(bytes);
    if embedded.is_empty() {
        return own;
    }
    // A reference with the same name is tried first: indexing the outlines
    // of every registered font costs a few milliseconds each, once.
    let all = references();
    let wanted = name.map(normalized_name);
    let named: Vec<Arc<ReferenceFont>> = all
        .iter()
        .filter(|r| wanted.is_some() && r.name().as_deref() == wanted.as_deref())
        .cloned()
        .collect();
    let mut best: Option<(usize, Arc<ReferenceFont>)> = None;
    let named_hit = named.iter().any(|r| {
        let shapes = r.shapes();
        embedded
            .iter()
            .filter(|(_, h)| shapes.contains_key(h))
            .count()
            * 2
            >= embedded.len()
    });
    let candidates = if named_hit { named } else { all };
    for r in candidates {
        let shapes = r.shapes();
        let hits = embedded
            .iter()
            .filter(|(_, h)| shapes.contains_key(h))
            .count();
        if hits > best.as_ref().map_or(0, |(n, _)| *n) {
            best = Some((hits, r));
        }
    }
    // Most glyphs must match, or this is a different font that happens to
    // share a few simple shapes (a dot, a bar).
    let Some((hits, reference)) = best.filter(|(n, _)| *n * 2 >= embedded.len()) else {
        return own;
    };
    let shapes = reference.shapes();
    let Some(mut table) = reference.recovery() else {
        return own;
    };
    // The subset holds exactly the glyphs the PDF uses: if the quick table
    // misses one, build the deep table (once per process).
    let missing = embedded.iter().any(|(_, h)| {
        shapes
            .get(h)
            .is_some_and(|r| !table.map.contains_key(r) || table.weak.contains(r))
    });
    if missing && let Some(deep) = reference.deep_recovery() {
        table = deep;
    }
    // The subset's own character map must agree with the reference where
    // both speak; LibreOffice writes a symbol map from its own codes, which
    // would name glyphs 'h' or 'x'.
    let mut out = own.unwrap_or_default();
    let (mut agree, mut checked) = (0, 0);
    for (gid, hash) in &embedded {
        if let (Some(mine), Some(theirs)) = (
            out.map.get(gid),
            shapes.get(hash).and_then(|r| table.map.get(r)),
        ) {
            checked += 1;
            agree += usize::from(mine.text == theirs.text);
        }
    }
    if checked > 0 && agree * 2 < checked {
        out.map.clear();
    }
    out.indic |= table.indic;
    let mut to_embedded: HashMap<u32, u32> = HashMap::with_capacity(hits);
    for (gid, hash) in &embedded {
        if let Some(&r) = shapes.get(hash) {
            to_embedded.entry(r).or_insert(*gid);
            if let Some(text) = table.map.get(&r) {
                // The subset's own map, once trusted, names glyphs whose
                // shapes are shared (‚ and , in Noto Serif) correctly.
                if !table.indic && out.map.contains_key(gid) {
                    continue;
                }
                out.map.insert(*gid, text.clone());
                if table.weak.contains(&r) {
                    out.weak.insert(*gid);
                }
            }
        }
    }
    for (run, text) in &table.sequences {
        let mapped: Option<Vec<u32>> = run.iter().map(|r| to_embedded.get(r).copied()).collect();
        if let Some(mapped) = mapped {
            out.sequences.insert(mapped, text.clone());
        }
    }
    out.max_sequence = out.sequences.keys().map(Vec::len).max().unwrap_or(0);
    out.blank = blank;
    out.complete = true;
    (!out.map.is_empty()).then_some(out)
}

/// A hash of every non-empty glyph outline in a font, in font units, and
/// the glyphs that draw nothing.
pub(crate) fn outline_hashes(data: &[u8]) -> (Vec<(u32, u64)>, HashSet<u32>) {
    let mut out = Vec::new();
    let mut blank = HashSet::new();
    let Ok(font) = FontRef::new(data) else {
        return (out, blank);
    };
    let count = font.maxp().map(|m| m.num_glyphs() as u32).unwrap_or(0);
    let outlines = font.outline_glyphs();
    let settings = || DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
    out.reserve(count as usize);
    for gid in 1..count {
        let Some(glyph) = outlines.get(GlyphId::new(gid)) else {
            continue;
        };
        let mut pen = HashPen::default();
        if glyph.draw(settings(), &mut pen).is_ok() {
            if pen.ops > 0 {
                out.push((gid, pen.hasher.finish()));
            } else {
                blank.insert(gid);
            }
        }
    }
    (out, blank)
}

/// Hashes a glyph outline. Horizontal position is taken from the first
/// point: TrueType outlines are shifted by the left side bearing when the
/// `head` flags say so, and producers that rewrite `hmtx` (Ghostscript)
/// move the same shape sideways. Vertical position is kept: it tells a
/// comma from a quote.
#[derive(Default)]
struct HashPen {
    hasher: DefaultHasher,
    ops: usize,
    origin_x: Option<f32>,
}

impl HashPen {
    fn op(&mut self, tag: u8, points: &[f32]) {
        self.ops += 1;
        tag.hash(&mut self.hasher);
        let ox = *self
            .origin_x
            .get_or_insert(points.first().copied().unwrap_or(0.0));
        for (i, p) in points.iter().enumerate() {
            let v = if i % 2 == 0 { p - ox } else { *p };
            (v.round() as i32).hash(&mut self.hasher);
        }
    }
}

impl OutlinePen for HashPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.op(0, &[x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.op(1, &[x, y]);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.op(2, &[cx0, cy0, x, y]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.op(3, &[cx0, cy0, cx1, cy1, x, y]);
    }
    fn close(&mut self) {
        self.op(4, &[]);
    }
}

/// Times building each table for every registered font (for tuning).
#[doc(hidden)]
pub fn print_build_times() {
    for f in references() {
        let t = std::time::Instant::now();
        let _ = f.shapes().len();
        let shapes = t.elapsed();
        let t = std::time::Instant::now();
        let _ = f.recovery().is_some();
        let quick = t.elapsed();
        let t = std::time::Instant::now();
        let _ = f.deep_recovery().is_some();
        let deep = t.elapsed();
        let t = std::time::Instant::now();
        let _ = f.pictures().len();
        let pictures = t.elapsed();
        println!(
            "{:40} shapes {:>8.1?} quick {:>8.1?} deep {:>8.1?} pictures {:>8.1?}",
            f.name().clone().unwrap_or_default(),
            shapes,
            quick,
            deep,
            pictures
        );
    }
}
