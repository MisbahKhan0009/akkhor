//! Recovering letters from glyphs by running the font's own shaping rules
//! forwards ("reverse shaping").
//!
//! A PDF's `ToUnicode` map is often wrong for Indic text: conjunct glyphs map
//! to nothing, reph glyphs disappear. But the embedded font still has its
//! character map and its OpenType substitution rules. Shaping candidate
//! syllables (`C + vowel sign`, `র্ + C`, `C ্ C`, ...) with that very font
//! shows which glyph each syllable becomes; a glyph that only appears for a
//! candidate maps back to that candidate's letters. The result is exact
//! for every syllable the font draws as a ligature, because it is the same
//! computation the PDF producer ran.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use akkhor_flow::script::{IndicCat, Script, indic_cat, script_of};
use harfrust::font::{Blob, Font};
use harfrust::{Buffer, Direction, ShapeOptions, ShapePlan, ShaperFont, shape};

/// How a glyph's letters relate to the glyphs around it in drawing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Ordinary: its letters come where it is drawn.
    Base,
    /// A vowel sign drawn before the consonant it follows in reading order
    /// (Bangla ি ে ৈ, Devanagari ि, ...).
    PreBase,
    /// A reph (র্ / र्) drawn after the syllable it starts.
    Reph,
    /// A pre-base vowel sign fused with a reph ("िर्" in र्ति): the vowel
    /// sign moves after its consonant, the reph before it.
    PreBaseReph,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphText {
    pub text: String,
    pub role: Role,
}

/// Glyph id → letters, for one font.
#[derive(Debug, Default)]
pub struct Recovery {
    pub map: HashMap<u32, GlyphText>,
    /// Glyph runs whose letters are not the sum of their glyphs' letters.
    pub sequences: HashMap<Vec<u32>, String>,
    /// Longest key in `sequences`.
    pub max_sequence: usize,
    /// Glyphs known only from fallbacks (substitution tables, glyph
    /// names), not from shaping: a deep build may do better.
    pub weak: HashSet<u32>,
    /// Glyphs that draw nothing (filled in for embedded subsets).
    pub blank: HashSet<u32>,
    /// Whether the font covers an Indic script (then recovered text is
    /// preferred over the PDF's own text layer).
    pub indic: bool,
}

/// Vowel signs that are drawn before their consonant.
pub fn is_prebase_matra(c: char) -> bool {
    matches!(
        c,
        '\u{09BF}' | '\u{09C7}' | '\u{09C8}' // Bengali ি ে ৈ
            | '\u{093F}' | '\u{094E}' // Devanagari ि, prishthamatra
            | '\u{0A3F}' // Gurmukhi ਿ
            | '\u{0ABF}' // Gujarati િ
            | '\u{0B47}' // Odia େ
            | '\u{0BC6}' | '\u{0BC7}' | '\u{0BC8}' // Tamil ெ ே ை
            | '\u{0D46}' | '\u{0D47}' | '\u{0D48}' // Malayalam െ േ ൈ
    )
}

fn ra_of(script: Script) -> Option<char> {
    Some(match script {
        Script::Bengali => 'র',
        Script::Devanagari => 'र',
        Script::Gurmukhi => 'ਰ',
        Script::Gujarati => 'ર',
        Script::Oriya => 'ର',
        Script::Telugu => 'ర',
        Script::Kannada => 'ರ',
        Script::Malayalam => 'ര',
        _ => return None,
    })
}

fn virama_of(script: Script) -> Option<char> {
    let base = match script {
        Script::Devanagari => 0x0900,
        Script::Bengali => 0x0980,
        Script::Gurmukhi => 0x0A00,
        Script::Gujarati => 0x0A80,
        Script::Oriya => 0x0B00,
        Script::Tamil => 0x0B80,
        Script::Telugu => 0x0C00,
        Script::Kannada => 0x0C80,
        Script::Malayalam => 0x0D00,
        _ => return None,
    };
    char::from_u32(base + 0x4D)
}

fn hr_script(s: Script) -> Option<harfrust::Script> {
    Some(match s {
        Script::Bengali => harfrust::Script::BENGALI,
        Script::Devanagari => harfrust::Script::DEVANAGARI,
        Script::Gurmukhi => harfrust::Script::GURMUKHI,
        Script::Gujarati => harfrust::Script::GUJARATI,
        Script::Oriya => harfrust::Script::ORIYA,
        Script::Tamil => harfrust::Script::TAMIL,
        Script::Telugu => harfrust::Script::TELUGU,
        Script::Kannada => harfrust::Script::KANNADA,
        Script::Malayalam => harfrust::Script::MALAYALAM,
        _ => return None,
    })
}

/// The role of a glyph whose letters are `text`, as far as the text alone
/// tells: a pre-base vowel sign (with candrabindu, anusvara or a reph fused
/// to it) or an ordinary glyph. Rephs drawn after their syllable are
/// recognized where the drawing order is known ([`role_of_ligature`]).
pub fn role_of(text: &str) -> Role {
    if let Some((_, rest)) = reph_split(text)
        && rest.chars().next().is_some_and(is_prebase_matra)
    {
        return Role::PreBaseReph;
    }
    let Some(first) = text.chars().next() else {
        return Role::Base;
    };
    if is_prebase_matra(first) {
        let marks = text
            .chars()
            .skip(1)
            .all(|c| matches!(indic_cat(c), Some((_, IndicCat::Sign | IndicCat::Nukta))));
        if marks {
            return Role::PreBase;
        }
    }
    Role::Base
}

/// Upper bound on candidate syllables shaped per font, so a huge font
/// cannot stall page loading.
const MAX_CANDIDATES: usize = 12_000;
/// The same for a deep build, which also tries three-consonant clusters.
const MAX_DEEP_CANDIDATES: usize = 80_000;

/// Builds the recovery table for a font. Returns `None` for font programs
/// the shaper cannot read (bare CFF, Type 1) or that map no letters.
pub fn build(data: &Arc<dyn AsRef<[u8]> + Send + Sync>) -> Option<Recovery> {
    build_with(data, true, false)
}

/// [`build`], optionally ignoring glyph names (many system fonts ship
/// without them; tests use this to prove shaping alone is enough), and
/// optionally deep: also shaping three-consonant clusters (স্ত্র, ত্র্য),
/// several times slower, for when a PDF uses a glyph the quick build missed.
pub fn build_with(
    data: &Arc<dyn AsRef<[u8]> + Send + Sync>,
    use_names: bool,
    deep: bool,
) -> Option<Recovery> {
    let blob: Blob = Blob::from(data.clone());
    let font = Font::new(blob, 0)?;
    let mut rec = Recovery::default();

    // 1. The character map, reversed.
    let mut chars_by_script: HashMap<Script, Vec<char>> = HashMap::new();
    for (cp, gid) in font.charmap().unicodes() {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        let gid = gid.to_u32();
        // Symbol-font maps (PDF subsets) put glyphs at U+F0xx: not letters.
        // Two-part vowels are drawn as their halves; a cmap entry for one
        // (ৌ on the ৗ glyph) is a fallback that would misname the half.
        if gid == 0 || !crate::text::usable(&c.to_string()) || is_two_part_vowel(c) {
            continue;
        }
        rec.map.entry(gid).or_insert_with(|| GlyphText {
            text: canonical(&c.to_string()),
            role: role_of(&c.to_string()),
        });
        let script = script_of(c);
        if script.is_indic() {
            chars_by_script.entry(script).or_default().push(c);
        }
    }
    if rec.map.is_empty() {
        return None;
    }
    rec.indic = !chars_by_script.is_empty();
    // Glyphs that draw nothing and stand for no character are what shapers
    // leave behind when a mark is drawn inside another glyph (Noto's
    // "NullMark" after ि fuses with ं). Known from the start as empty, so
    // the fused glyph gets all the letters.
    if rec.indic {
        for gid in crate::reference::outline_hashes((**data).as_ref()).1 {
            rec.map.entry(gid).or_insert(GlyphText {
                text: String::new(),
                role: Role::Base,
            });
        }
    }
    for chars in chars_by_script.values_mut() {
        chars.sort_unstable();
        chars.dedup();
    }

    // 2. Shape candidate syllables per script and learn new glyphs.
    let shaper = ShaperFont::new(&font);
    let mut buffer = Buffer::new();
    let mut budget = if deep {
        MAX_DEEP_CANDIDATES
    } else {
        MAX_CANDIDATES
    };
    // Glyph runs some candidate explained glyph by glyph: a sequence with
    // the same glyphs is another spelling of the same shape (Bangla র্ and
    // Assamese ৰ্ share the reph), not a special case.
    let mut explained: HashSet<Vec<u32>> = HashSet::new();
    let mut scripts: Vec<_> = chars_by_script.into_iter().collect();
    scripts.sort_by_key(|(s, _)| *s as u8);
    for (script, chars) in &scripts {
        let Some(hs) = hr_script(*script) else {
            continue;
        };
        let plan = ShapePlan::new(&font, Direction::LeftToRight, Some(hs), None, &[]);
        let of = |k: IndicCat| -> Vec<char> {
            chars
                .iter()
                .copied()
                .filter(|c| indic_cat(*c).map(|(_, x)| x) == Some(k))
                .collect()
        };
        let consonants = of(IndicCat::Consonant);
        let vowels = of(IndicCat::Vowel);
        let matras = of(IndicCat::Matra);
        let signs: Vec<char> = of(IndicCat::Sign)
            .into_iter()
            .filter(|c| matches!(*c as u32 & 0x7F, 0x01 | 0x02))
            .collect(); // candrabindu, anusvara
        let nukta = of(IndicCat::Nukta).first().copied();
        let virama = virama_of(*script).filter(|v| chars.contains(v));
        let ra = ra_of(*script).filter(|r| chars.contains(r));

        // Shapes one candidate; returns whether it taught something new and
        // whether the font drew it as a single glyph.
        let mut shape_one =
            |text: String, rec: &mut Recovery, budget: &mut usize| -> (bool, bool) {
                if *budget == 0 {
                    return (false, false);
                }
                *budget -= 1;
                buffer.clear();
                buffer.push_str(&text);
                buffer.set_direction(Direction::LeftToRight);
                buffer.set_script(Some(hs));
                if shape(&shaper, &mut buffer, ShapeOptions::new().plan(Some(&plan))).is_err() {
                    return (false, false);
                }
                let gids: Vec<u32> = buffer.glyph_infos().iter().map(|g| g.glyph_id).collect();
                let learned = learn(&text, &gids, rec);
                let single = gids.len() == 1;
                if learned || !remember_sequence(&text, &gids, rec) {
                    explained.insert(gids);
                }
                (learned, single)
            };

        // Reph first: its glyph also appears inside other shapes (Noto
        // draws the top of ई with it), and must be known as a reph.
        if let (Some(r), Some(v)) = (ra, virama) {
            for &c in &consonants {
                shape_one(format!("{r}{v}{c}"), &mut rec, &mut budget);
            }
            for &c in &consonants {
                for &m in &matras {
                    shape_one(format!("{r}{v}{c}{m}"), &mut rec, &mut budget);
                }
            }
        }
        // Vowel signs, nukta and their combinations with candrabindu and
        // anusvara (ें, ैं, ों are single glyphs in many fonts).
        for &c in &consonants {
            for &m in &matras {
                shape_one(format!("{c}{m}"), &mut rec, &mut budget);
                for &b in &signs {
                    shape_one(format!("{c}{m}{b}"), &mut rec, &mut budget);
                }
            }
            if let Some(n) = nukta {
                shape_one(format!("{c}{n}"), &mut rec, &mut budget);
            }
        }
        for &w in &vowels {
            shape_one(w.to_string(), &mut rec, &mut budget);
            for &b in &signs {
                shape_one(format!("{w}{b}"), &mut rec, &mut budget);
            }
        }
        let Some(v) = virama else { continue };
        // Two-consonant conjuncts and phala forms; remember real ligatures
        // (drawn as one glyph, or teaching a new one).
        let mut ligatures: Vec<String> = Vec::new();
        for &a in &consonants {
            for &b in &consonants {
                let s = format!("{a}{v}{b}");
                let (learned, single) = shape_one(s.clone(), &mut rec, &mut budget);
                if learned || single {
                    ligatures.push(s);
                }
            }
        }
        // Conjuncts with vowel signs and rephs, then three-consonant
        // clusters built on the ligatures. The quick build only tries those
        // ending in a phala (স্ত্র, ত্র্য, ন্দ্ব), by far the common ones.
        let phala = |c: char| {
            matches!(c as u32 & 0x7F, 0x2F | 0x30 | 0x35 | 0x2E | 0x32 | 0x28) // য র ব ম ল ন
        };
        let mut seen: HashSet<String> = HashSet::new();
        for lig in &ligatures {
            for &m in &matras {
                shape_one(format!("{lig}{m}"), &mut rec, &mut budget);
            }
            if let Some(r) = ra {
                shape_one(format!("{r}{v}{lig}"), &mut rec, &mut budget);
            }
        }
        for lig in &ligatures {
            let ends_in_phala = lig.chars().last().is_some_and(phala);
            for &c in &consonants {
                if deep || ends_in_phala {
                    let s = format!("{c}{v}{lig}");
                    if seen.insert(s.clone()) {
                        shape_one(s, &mut rec, &mut budget);
                    }
                }
                if deep || phala(c) {
                    let s = format!("{lig}{v}{c}");
                    if seen.insert(s.clone()) {
                        shape_one(s, &mut rec, &mut budget);
                    }
                }
            }
        }
    }
    // 3. Glyph names like "uni09470902" (ें) say what a glyph is, when the
    //    font kept them. Only a fallback: designers share one glyph between
    //    letters (Bangla র্ and Assamese ৰ্), which shaping tells apart.
    if use_names && let Ok(f) = skrifa::FontRef::new((**data).as_ref()) {
        use skrifa::raw::TableProvider;
        if let Ok(post) = f.post() {
            let count = f.maxp().map(|m| m.num_glyphs()).unwrap_or(0);
            for gid in 1..count {
                if rec.map.contains_key(&(gid as u32)) {
                    continue;
                }
                let text = post
                    .glyph_name(skrifa::raw::types::GlyphId16::new(gid))
                    .and_then(text_of_glyph_name);
                if let Some(text) = text.filter(|t| t.chars().any(|c| script_of(c).is_indic())) {
                    let role = role_of_ligature(&text);
                    rec.map.insert(gid as u32, GlyphText { text, role });
                    rec.weak.insert(gid as u32);
                }
            }
        }
    }

    // 4. Whatever is still unknown, from the substitution tables
    //    directly: a ligature means its components' letters, an alternate
    //    means its original's. Reaches conjuncts no candidate spelled out
    //    (त्त्व, न्द्र in fonts that build them in one step).
    learn_from_gsub((**data).as_ref(), &mut rec);
    rec.sequences.retain(|run, _| !explained.contains(run));
    rec.max_sequence = rec.sequences.keys().map(Vec::len).max().unwrap_or(0);
    Some(rec)
}

/// Text in its decomposed form: the nukta letters U+09DC, U+09DD, U+09DF
/// and U+0958..U+095F are composition exclusions, so NFC text spells them
/// as letter + nukta.
pub fn canonical(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{09DC}' => out.push_str("\u{09A1}\u{09BC}"),
            '\u{09DD}' => out.push_str("\u{09A2}\u{09BC}"),
            '\u{09DF}' => out.push_str("\u{09AF}\u{09BC}"),
            '\u{0958}'..='\u{095F}' => {
                const BASE: [char; 8] = ['क', 'ख', 'ग', 'ज', 'ड', 'ढ', 'फ', 'य'];
                out.push(BASE[(c as u32 - 0x0958) as usize]);
                out.push('\u{093C}');
            }
            c => out.push(c),
        }
    }
    out
}

/// Letters of a glyph name in the Adobe Glyph List style: "uni0915094D",
/// "u1F600", components joined by '_', any ".suffix" ignored.
fn text_of_glyph_name(name: &str) -> Option<String> {
    let name = name.split('.').next()?;
    let mut out = String::new();
    for part in name.split('_') {
        let hex = if let Some(h) = part.strip_prefix("uni") {
            if h.is_empty() || h.len() % 4 != 0 {
                return None;
            }
            h.as_bytes()
                .chunks(4)
                .map(|c| {
                    std::str::from_utf8(c)
                        .ok()
                        .and_then(|c| u32::from_str_radix(c, 16).ok())
                })
                .collect::<Option<Vec<u32>>>()?
        } else {
            let h = part
                .strip_prefix('u')
                .filter(|h| (4..=6).contains(&h.len()))?;
            vec![u32::from_str_radix(h, 16).ok()?]
        };
        for cp in hex {
            let c = char::from_u32(cp)?;
            if c.is_control() {
                return None;
            }
            out.push(c);
        }
    }
    (!out.is_empty()).then(|| canonical(&out))
}

/// The role of a ligature glyph: a reph, alone or fused with the vowel
/// sign under it ("র্ী" or, in glyph-name order, "ীর্"), moves to the start
/// of its syllable.
fn role_of_ligature(text: &str) -> Role {
    match role_of(text) {
        Role::Base if reph_split(text).is_some() => Role::Reph,
        role => role,
    }
}

/// Splits a reph glyph's text into (reph, rest): the ra + virama at its
/// start or end, and the vowel signs fused with it. `None` when the text
/// has a consonant besides the ra (a conjunct, not a reph).
pub fn reph_split(text: &str) -> Option<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let is_virama = |c: char| matches!(indic_cat(c), Some((_, IndicCat::Virama)));
    let (reph, rest): (&[char], &[char]) = if chars.len() >= 2
        && is_ra(chars[0])
        && is_virama(chars[1])
    {
        (&chars[..2], &chars[2..])
    } else if chars.len() >= 2 && is_ra(chars[chars.len() - 2]) && is_virama(chars[chars.len() - 1])
    {
        (&chars[chars.len() - 2..], &chars[..chars.len() - 2])
    } else {
        return None;
    };
    let fused_ok = rest
        .iter()
        .all(|&c| matches!(indic_cat(c), Some((_, IndicCat::Matra | IndicCat::Sign))));
    fused_ok.then(|| (reph.iter().collect(), rest.iter().collect()))
}

/// Letters as a sorted multiset, with two-part vowels split, for checking
/// whether glyphs explain a candidate up to reordering.
fn letter_bag(text: &str) -> Vec<char> {
    let folded = akkhor_flow::search::fold(text).text;
    let mut bag = Vec::new();
    for c in folded.chars() {
        match c {
            '\u{094B}' => bag.extend(['\u{093E}', '\u{0947}']), // ो = ा + े
            '\u{094C}' => bag.extend(['\u{093E}', '\u{0948}']), // ौ = ा + ै
            '\u{0949}' => bag.extend(['\u{093E}', '\u{0945}']), // ॉ = ा + ॅ
            '\u{094A}' => bag.extend(['\u{093E}', '\u{0946}']), // ॊ = ा + ॆ
            '\u{093B}' => bag.extend(['\u{093E}', '\u{093A}']), // ऻ = ा + ऺ
            '\u{200C}' | '\u{200D}' => {}
            c => bag.push(c),
        }
    }
    bag.sort_unstable();
    bag
}

/// Records a glyph sequence that single glyphs cannot explain (ई drawn as
/// इ plus the reph glyph), so the page reader can match it as a whole.
/// Returns whether it recorded one.
fn remember_sequence(text: &str, gids: &[u32], rec: &mut Recovery) -> bool {
    if gids.len() < 2 || gids.len() > 4 || gids.contains(&0) {
        return false;
    }
    let mut parts = String::new();
    for g in gids {
        match rec.map.get(g) {
            Some(t) => parts.push_str(&t.text),
            None => return false,
        }
    }
    if letter_bag(&parts) == letter_bag(text) {
        return false;
    }
    rec.sequences
        .entry(gids.to_vec())
        .or_insert_with(|| canonical(text));
    true
}

/// Learns glyphs from one shaped candidate. Glyphs already known explain
/// some of the letters; a single unknown glyph takes the rest. Returns true
/// when something new was learned.
fn learn(text: &str, gids: &[u32], rec: &mut Recovery) -> bool {
    let mut unknown: Option<usize> = None;
    for (i, gid) in gids.iter().enumerate() {
        if rec.map.contains_key(gid) {
            continue;
        }
        if *gid == 0 || unknown.is_some() {
            return false; // .notdef (the font cannot draw this) or ambiguous
        }
        unknown = Some(i);
    }
    let Some(at) = unknown else { return false };
    // Known glyphs explain some letters: those drawn before the unknown one
    // are taken from the front of the text, those after from the back, so
    // in "क्क" the half form gets "क्", not "्क". A glyph drawn out of
    // order (a pre-base vowel sign, a reph) falls back to any occurrence.
    let mut residue: Vec<Option<char>> = text.chars().map(Some).collect();
    let take = |residue: &mut Vec<Option<char>>, letters: &str, from_front: bool| {
        let live: Vec<usize> = (0..residue.len())
            .filter(|&i| residue[i].is_some())
            .collect();
        let want: Vec<char> = letters.chars().collect();
        let window = if from_front {
            live.get(..want.len())
        } else {
            live.len().checked_sub(want.len()).map(|s| &live[s..])
        };
        if let Some(w) = window
            && w.iter().zip(&want).all(|(&i, &c)| residue[i] == Some(c))
        {
            for &i in w {
                residue[i] = None;
            }
            return;
        }
        for c in want {
            let pos = if from_front {
                residue.iter().rposition(|&r| r == Some(c))
            } else {
                residue.iter().position(|&r| r == Some(c))
            };
            if let Some(p) = pos {
                residue[p] = None;
            }
        }
    };
    for gid in &gids[..at] {
        let known = rec.map[gid].text.clone();
        take(&mut residue, &known, true);
    }
    for gid in gids[at + 1..].iter().rev() {
        let known = rec.map[gid].text.clone();
        take(&mut residue, &known, false);
    }
    let residue: Vec<char> = residue.into_iter().flatten().collect();
    if residue.is_empty() {
        return false;
    }
    let letters: String = canonical(&residue.into_iter().collect::<String>());
    let first_base = gids.iter().position(|g| {
        rec.map
            .get(g)
            .is_some_and(|t| t.role == Role::Base && starts_with_letter(&t.text))
    });
    // A reph, alone or fused with a vowel sign ("র্ী"), drawn after the
    // consonant it belongs before.
    let role = if role_of_ligature(&letters) == Role::Reph && first_base.is_some_and(|b| b < at) {
        Role::Reph
    } else {
        role_of(&letters)
    };
    rec.map.insert(
        gids[at],
        GlyphText {
            text: letters,
            role,
        },
    );
    true
}

fn is_ra(c: char) -> bool {
    matches!(c, 'র' | 'र' | 'ਰ' | 'ર' | 'ର' | 'ర' | 'ರ' | 'ര')
}

/// Whether text starts with a consonant or independent vowel.
pub fn starts_with_letter(text: &str) -> bool {
    text.chars().next().is_some_and(|c| match indic_cat(c) {
        Some((_, k)) => matches!(k, IndicCat::Consonant | IndicCat::Vowel),
        None => c.is_alphanumeric(),
    })
}

/// Fills in glyphs from the font's GSUB lookups, ignoring their context,
/// until nothing new is learned.
fn learn_from_gsub(data: &[u8], rec: &mut Recovery) {
    use skrifa::raw::TableProvider;
    use skrifa::raw::tables::gsub::{
        AlternateSubst, LigatureSubst, SingleSubst, SubstitutionSubtables,
    };
    let Ok(font) = skrifa::FontRef::new(data) else {
        return;
    };
    let Ok(gsub) = font.gsub() else { return };
    let Ok(lookups) = gsub.lookup_list() else {
        return;
    };

    // Lookups that form rephs.
    let mut reph_lookups: HashSet<u16> = HashSet::new();
    if let Ok(features) = gsub.feature_list() {
        for record in features.feature_records() {
            if record.feature_tag() == skrifa::Tag::new(b"rphf")
                && let Ok(feature) = record.feature(features.offset_data())
            {
                reph_lookups.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
            }
        }
    }

    // (output glyph, input glyphs, forms a reph)
    let mut rules: Vec<(u32, Vec<u32>, bool)> = Vec::new();
    for (index, lookup) in lookups.lookups().iter().enumerate() {
        let Ok(lookup) = lookup else { continue };
        let reph = reph_lookups.contains(&(index as u16));
        let Ok(subtables) = lookup.subtables() else {
            continue;
        };
        match subtables {
            SubstitutionSubtables::Single(tables) => {
                for t in tables.iter().flatten() {
                    match t {
                        SingleSubst::Format1(t) => {
                            let Ok(cov) = t.coverage() else { continue };
                            for g in cov.iter() {
                                let to = (g.to_u32() as i32 + t.delta_glyph_id() as i32) as u16;
                                rules.push((to as u32, vec![g.to_u32()], reph));
                            }
                        }
                        SingleSubst::Format2(t) => {
                            let Ok(cov) = t.coverage() else { continue };
                            for (g, to) in cov.iter().zip(t.substitute_glyph_ids()) {
                                rules.push((to.get().to_u32(), vec![g.to_u32()], reph));
                            }
                        }
                        // 24-bit glyph ids: fonts past 65 535 glyphs, not Indic.
                        _ => {}
                    }
                }
            }
            SubstitutionSubtables::Alternate(tables) => {
                for t in tables.iter().flatten() {
                    let AlternateSubst::Format1(t) = t else {
                        continue;
                    };
                    let Ok(cov) = t.coverage() else { continue };
                    for (g, set) in cov.iter().zip(t.alternate_sets().iter()) {
                        let Ok(set) = set else { continue };
                        for to in set.alternate_glyph_ids() {
                            rules.push((to.get().to_u32(), vec![g.to_u32()], reph));
                        }
                    }
                }
            }
            SubstitutionSubtables::Ligature(tables) => {
                for t in tables.iter().flatten() {
                    let LigatureSubst::Format1(t) = t else {
                        continue;
                    };
                    let Ok(cov) = t.coverage() else { continue };
                    for (first, set) in cov.iter().zip(t.ligature_sets().iter()) {
                        let Ok(set) = set else { continue };
                        for lig in set.ligatures().iter().flatten() {
                            let mut parts = vec![first.to_u32()];
                            parts
                                .extend(lig.component_glyph_ids().iter().map(|g| g.get().to_u32()));
                            rules.push((lig.ligature_glyph().to_u32(), parts, reph));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for _ in 0..8 {
        let mut learned = false;
        for (out, parts, reph) in &rules {
            if *out == 0 || rec.map.contains_key(out) {
                continue;
            }
            let Some(texts) = parts
                .iter()
                .map(|g| rec.map.get(g))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let mut text = String::new();
            for t in &texts {
                // A half form's virama and a below-base form's own virama
                // are one virama in the text (त् + ्र is त्र).
                let is_virama = |c: char| matches!(indic_cat(c), Some((_, IndicCat::Virama)));
                if text.chars().last().is_some_and(is_virama)
                    && t.text.chars().next().is_some_and(is_virama)
                {
                    text.pop();
                }
                text.push_str(&t.text);
            }
            if text.is_empty() {
                continue;
            }
            let role = if *reph {
                Role::Reph
            } else if parts.len() == 1 {
                texts[0].role
            } else {
                // A reph fused with the vowel sign drawn under it.
                let fused = reph_split(&text).is_some() && !text.chars().next().is_some_and(is_ra);
                match role_of(&text) {
                    Role::Base if fused => Role::Reph,
                    role => role,
                }
            };
            rec.map.insert(*out, GlyphText { text, role });
            rec.weak.insert(*out);
            learned = true;
        }
        if !learned {
            break;
        }
    }
}

/// Vowel signs with a canonical two-part decomposition.
fn is_two_part_vowel(c: char) -> bool {
    matches!(
        c,
        '\u{09CB}' | '\u{09CC}' // Bengali ো ৌ
            | '\u{0B48}' | '\u{0B4B}' | '\u{0B4C}' // Odia
            | '\u{0BCA}' | '\u{0BCB}' | '\u{0BCC}' // Tamil
            | '\u{0C48}' // Telugu
            | '\u{0CC0}' | '\u{0CC7}' | '\u{0CC8}' | '\u{0CCA}' | '\u{0CCB}' // Kannada
            | '\u{0D4A}' | '\u{0D4B}' | '\u{0D4C}' // Malayalam
    )
}
