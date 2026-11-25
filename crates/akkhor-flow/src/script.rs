//! Scripts and orthographic syllables.
//!
//! Indic scripts (Bangla, Devanagari, Gurmukhi, Gujarati, Odia, Tamil,
//! Telugu, Kannada, Malayalam) build syllables from several code points: a
//! consonant, a virama (hasant/halant) and another consonant join into one
//! conjunct; vowel signs and marks attach to it. W3C's layout requirements
//! say such a syllable is "an indivisible unit of text for most editing
//! operations", and it can be longer than a Unicode grapheme cluster.
//!
//! [`syllables`] finds those units. Line breaking falls back to them for
//! words too long for a line, and selection/highlighting snap to them, so a
//! conjunct such as ক্ষ, ন্ত্র or द्ध is never cut in half.
//!
//! All nine scripts share the ISCII-derived layout of their Unicode blocks,
//! so one table, offset by the block start, classifies all of them.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Script {
    /// Spaces, punctuation, digits, joiners: takes the script around it.
    Common,
    Latin,
    Bengali,
    Devanagari,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    /// Anything else (Arabic, CJK, ...), shaped with fallback fonts.
    Other,
}

impl Script {
    pub fn is_indic(self) -> bool {
        matches!(
            self,
            Script::Bengali
                | Script::Devanagari
                | Script::Gurmukhi
                | Script::Gujarati
                | Script::Oriya
                | Script::Tamil
                | Script::Telugu
                | Script::Kannada
                | Script::Malayalam
        )
    }

    /// ISO 15924 code.
    pub fn iso15924(self) -> &'static str {
        match self {
            Script::Common => "Zyyy",
            Script::Latin => "Latn",
            Script::Bengali => "Beng",
            Script::Devanagari => "Deva",
            Script::Gurmukhi => "Guru",
            Script::Gujarati => "Gujr",
            Script::Oriya => "Orya",
            Script::Tamil => "Taml",
            Script::Telugu => "Telu",
            Script::Kannada => "Knda",
            Script::Malayalam => "Mlym",
            Script::Other => "Zzzz",
        }
    }

    fn block_base(self) -> Option<u32> {
        Some(match self {
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
        })
    }

    /// Whether virama + consonant forms a conjunct (one syllable). In Tamil
    /// the pulli usually shows as a visible dead consonant, so the next
    /// consonant starts a new syllable.
    fn virama_joins(self) -> bool {
        self != Script::Tamil
    }
}

/// The script of one character.
pub fn script_of(c: char) -> Script {
    let cp = c as u32;
    match cp {
        0x0900..=0x0963 | 0x0966..=0x097F | 0xA8E0..=0xA8FF => Script::Devanagari,
        // Dandas U+0964/0965 are shared by all Indic scripts.
        0x0964..=0x0965 => Script::Common,
        0x0980..=0x09FF => Script::Bengali,
        0x0A00..=0x0A7F => Script::Gurmukhi,
        0x0A80..=0x0AFF => Script::Gujarati,
        0x0B00..=0x0B7F => Script::Oriya,
        0x0B80..=0x0BFF => Script::Tamil,
        0x0C00..=0x0C7F => Script::Telugu,
        0x0C80..=0x0CFF => Script::Kannada,
        0x0D00..=0x0D7F => Script::Malayalam,
        0x200C | 0x200D => Script::Common,
        _ if c.is_ascii_alphabetic() => Script::Latin,
        _ if c.is_ascii() => Script::Common,
        0x00A0..=0x00BF | 0x00D7 | 0x00F7 | 0x2000..=0x206F | 0x20A0..=0x20CF => Script::Common,
        0x0300..=0x036F => Script::Common,
        0x00C0..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        _ => Script::Other,
    }
}

/// The role of a character inside an Indic syllable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicCat {
    Consonant,
    /// Independent vowel letter (starts a syllable).
    Vowel,
    /// Dependent vowel sign (matra / kar).
    Matra,
    /// Hasant / halant / pulli / chandrakkala.
    Virama,
    Nukta,
    /// Candrabindu, anusvara, visarga and other combining signs.
    Sign,
    Digit,
    Other,
}

/// Classifies an Indic character, or `None` outside the nine Indic blocks.
pub fn indic_cat(c: char) -> Option<(Script, IndicCat)> {
    let script = script_of(c);
    let base = script.block_base()?;
    let off = c as u32 - base;
    use IndicCat::*;
    // Script-specific code points that break the shared pattern.
    let special = match (script, off) {
        (Script::Bengali, 0x4E) => Some(Consonant), // ৎ khanda ta
        (Script::Bengali, 0x70..=0x71) => Some(Consonant), // Assamese ra, wa
        (Script::Bengali, 0x7E) => Some(Sign),      // sandhi mark
        (Script::Devanagari, 0x72..=0x77) => Some(Vowel),
        (Script::Devanagari, 0x78..=0x7F) => Some(Consonant),
        (Script::Devanagari, 0x71) => Some(Other),
        (Script::Gurmukhi, 0x70..=0x71) => Some(Sign), // tippi, addak
        (Script::Gurmukhi, 0x72..=0x73) => Some(Vowel),
        (Script::Gurmukhi, 0x75) => Some(Sign),  // yakash
        (Script::Malayalam, 0x4E) => Some(Sign), // dot reph
        (Script::Malayalam, 0x54..=0x56) => Some(Consonant), // chillus
        (Script::Malayalam, 0x7A..=0x7F) => Some(Consonant), // chillus
        _ => None,
    };
    let cat = special.unwrap_or(match off {
        0x00..=0x03 => Sign,
        0x04..=0x14 => Vowel,
        0x15..=0x39 => Consonant,
        0x3A..=0x3B => Matra,
        0x3C => Nukta,
        0x3D => Vowel, // avagraha behaves like a base
        0x3E..=0x4C => Matra,
        0x4D => Virama,
        0x4E..=0x4F => Matra,
        0x51..=0x54 => Sign,
        0x55..=0x57 => Matra,
        0x58..=0x5F => Consonant,
        0x60..=0x61 => Vowel,
        0x62..=0x63 => Matra,
        0x66..=0x6F => Digit,
        _ => Other,
    });
    Some((script, cat))
}

const ZWNJ: char = '\u{200C}';
const ZWJ: char = '\u{200D}';

fn is_combining_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
        || matches!(
            indic_cat(c),
            Some((
                _,
                IndicCat::Matra | IndicCat::Virama | IndicCat::Nukta | IndicCat::Sign
            ))
        )
}

/// Whether a syllable boundary may fall between `prev` and `cur`, where
/// `before_prev` is the character before `prev` (if any).
pub fn is_syllable_boundary(before_prev: Option<char>, prev: char, cur: char) -> bool {
    // Marks and joiners always attach to what precedes them.
    if is_combining_mark(cur) || cur == ZWJ || cur == ZWNJ {
        return false;
    }
    let cur_cat = indic_cat(cur);
    let is_consonant = |c: Option<(Script, IndicCat)>| matches!(c, Some((_, IndicCat::Consonant)));

    // Consonant + virama + consonant: a conjunct.
    if let Some((script, IndicCat::Virama)) = indic_cat(prev)
        && is_consonant(cur_cat)
        && script.virama_joins()
    {
        return false;
    }
    // Consonant + virama + ZWJ/ZWNJ + consonant: an explicit half form
    // (Bangla ত্‍ for khanda ta, Devanagari half forms).
    if (prev == ZWJ || prev == ZWNJ)
        && let Some(bp) = before_prev
        && matches!(indic_cat(bp), Some((_, IndicCat::Virama)))
        && is_consonant(cur_cat)
    {
        return prev == ZWNJ;
    }
    true
}

/// Byte ranges of the orthographic syllables in `text`. Non-Indic text comes
/// out as one range per character plus its combining marks.
pub fn syllables(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut before_prev: Option<char> = None;
    let mut prev: Option<char> = None;
    for (i, c) in text.char_indices() {
        if let Some(p) = prev
            && is_syllable_boundary(before_prev, p, c)
        {
            out.push(start..i);
            start = i;
        }
        before_prev = prev;
        prev = Some(c);
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

/// The byte offsets in `text` where a syllable starts: the positions a
/// cursor, selection edge or emergency line break may use.
pub fn syllable_starts(text: &str) -> Vec<usize> {
    syllables(text).into_iter().map(|r| r.start).collect()
}

/// Splits `text` into runs of one script. Common characters (spaces,
/// punctuation, joiners) join the run before them, or the one after them at
/// the start of the text.
pub fn script_runs(text: &str) -> Vec<(Range<usize>, Script)> {
    let mut runs: Vec<(Range<usize>, Script)> = Vec::new();
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        let s = script_of(c);
        match runs.last_mut() {
            Some((range, rs)) if s == Script::Common || s == *rs => range.end = end,
            Some((range, rs)) if *rs == Script::Common => {
                // A run that so far is all common takes the first real script.
                *rs = s;
                range.end = end;
            }
            _ => runs.push((i..end, s)),
        }
    }
    // Leading whitespace before a script change belongs with the next run's
    // text visually, but keeping it with the previous one is harmless; only a
    // document that is entirely Common keeps Script::Common.
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(s: &str) -> Vec<&str> {
        syllables(s).into_iter().map(|r| &s[r]).collect()
    }

    #[test]
    fn bangla_conjuncts_stay_whole() {
        // ক্ষ (k + hasant + ss), then মা
        assert_eq!(split("ক্ষমা"), vec!["ক্ষ", "মা"]);
        // ন্ত্র: three consonants in one conjunct
        assert_eq!(split("মন্ত্রী"), vec!["ম", "ন্ত্রী"]);
        // reph: র্ + ক
        assert_eq!(split("কর্ম"), vec!["ক", "র্ম"]);
        // ya-phala and candrabindu
        assert_eq!(split("ব্যাঁক"), vec!["ব্যাঁ", "ক"]);
        // khanda ta is a full consonant
        assert_eq!(split("উৎসব"), vec!["উ", "ৎ", "স", "ব"]);
        // ZWNJ after hasant keeps the hasant visible: two syllables
        assert_eq!(split("ক্\u{200C}ষ"), vec!["ক্\u{200C}", "ষ"]);
    }

    #[test]
    fn devanagari_conjuncts_stay_whole() {
        assert_eq!(split("शुद्ध"), vec!["शु", "द्ध"]);
        assert_eq!(split("क्षत्रिय"), vec!["क्ष", "त्रि", "य"]);
        assert_eq!(split("श्री"), vec!["श्री"]);
        assert_eq!(split("हिन्दी"), vec!["हि", "न्दी"]);
        // nukta attaches
        assert_eq!(split("ज़रा"), vec!["ज़", "रा"]);
    }

    #[test]
    fn other_indic_scripts() {
        // Gujarati, Telugu, Kannada and Malayalam conjuncts
        assert_eq!(split("ક્ષ"), vec!["ક્ષ"]);
        assert_eq!(split("క్ష"), vec!["క్ష"]);
        assert_eq!(split("ಕ್ಷ"), vec!["ಕ್ಷ"]);
        assert_eq!(split("ക്ഷ"), vec!["ക്ഷ"]);
        // Tamil pulli does not join
        assert_eq!(split("க்க"), vec!["க்", "க"]);
    }

    #[test]
    fn latin_and_marks() {
        assert_eq!(split("ae\u{301}"), vec!["a", "e\u{301}"]);
    }

    #[test]
    fn runs_by_script() {
        let t = "Hello আমার বাংলা and हिन्दी।";
        let runs: Vec<_> = script_runs(t)
            .into_iter()
            .map(|(r, s)| (&t[r], s))
            .collect();
        assert_eq!(
            runs,
            vec![
                ("Hello ", Script::Latin),
                ("আমার বাংলা ", Script::Bengali),
                ("and ", Script::Latin),
                ("हिन्दी।", Script::Devanagari),
            ]
        );
    }
}
