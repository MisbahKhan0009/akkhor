//! Decoding plain-text books, and spotting legacy Bijoy-encoded Bangla.
//!
//! Before Unicode, most Bangla was typed with Bijoy and shown with fonts
//! such as SutonnyMJ: the file holds Latin letters and symbols (`Avwg` for
//! আমি, `†` for the e-kar that comes first) and only the font makes them look
//! Bangla. Without that font such a file is gibberish, so the engine detects
//! it and tells the app. Converting it to Unicode is part of Phase 2.

/// How a text file was encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    /// Windows-1252 (Western European), the usual 8-bit fallback.
    Windows1252,
    /// Bangla typed with Bijoy (SutonnyMJ and similar fonts). The decoded
    /// text has already been converted to Unicode.
    LegacyBijoy,
}

#[derive(Clone, Debug)]
pub struct DecodedText {
    pub text: String,
    pub encoding: TextEncoding,
}

/// Windows-1252 code points for bytes 0x80..=0x9F (0 where undefined).
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
    0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// Windows-1251 (Cyrillic), bytes 0x80 to 0xFF.
const CP1251: [u16; 128] = [
    0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409, 0x2039,
    0x040A, 0x040C, 0x040B, 0x040F, 0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0xFFFD, 0x2122, 0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F, 0x00A0, 0x040E, 0x045E, 0x0408,
    0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9, 0x0404, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x0407,
    0x00B0, 0x00B1, 0x0406, 0x0456, 0x0491, 0x00B5, 0x00B6, 0x00B7, 0x0451, 0x2116, 0x0454, 0x00BB,
    0x0458, 0x0405, 0x0455, 0x0457, 0x0410, 0x0411, 0x0412, 0x0413, 0x0414, 0x0415, 0x0416, 0x0417,
    0x0418, 0x0419, 0x041A, 0x041B, 0x041C, 0x041D, 0x041E, 0x041F, 0x0420, 0x0421, 0x0422, 0x0423,
    0x0424, 0x0425, 0x0426, 0x0427, 0x0428, 0x0429, 0x042A, 0x042B, 0x042C, 0x042D, 0x042E, 0x042F,
    0x0430, 0x0431, 0x0432, 0x0433, 0x0434, 0x0435, 0x0436, 0x0437, 0x0438, 0x0439, 0x043A, 0x043B,
    0x043C, 0x043D, 0x043E, 0x043F, 0x0440, 0x0441, 0x0442, 0x0443, 0x0444, 0x0445, 0x0446, 0x0447,
    0x0448, 0x0449, 0x044A, 0x044B, 0x044C, 0x044D, 0x044E, 0x044F,
];

/// KOI8-R (Cyrillic), bytes 0x80 to 0xFF.
const KOI8R: [u16; 128] = [
    0x2500, 0x2502, 0x250C, 0x2510, 0x2514, 0x2518, 0x251C, 0x2524, 0x252C, 0x2534, 0x253C, 0x2580,
    0x2584, 0x2588, 0x258C, 0x2590, 0x2591, 0x2592, 0x2593, 0x2320, 0x25A0, 0x2219, 0x221A, 0x2248,
    0x2264, 0x2265, 0x00A0, 0x2321, 0x00B0, 0x00B2, 0x00B7, 0x00F7, 0x2550, 0x2551, 0x2552, 0x0451,
    0x2553, 0x2554, 0x2555, 0x2556, 0x2557, 0x2558, 0x2559, 0x255A, 0x255B, 0x255C, 0x255D, 0x255E,
    0x255F, 0x2560, 0x2561, 0x0401, 0x2562, 0x2563, 0x2564, 0x2565, 0x2566, 0x2567, 0x2568, 0x2569,
    0x256A, 0x256B, 0x256C, 0x00A9, 0x044E, 0x0430, 0x0431, 0x0446, 0x0434, 0x0435, 0x0444, 0x0433,
    0x0445, 0x0438, 0x0439, 0x043A, 0x043B, 0x043C, 0x043D, 0x043E, 0x043F, 0x044F, 0x0440, 0x0441,
    0x0442, 0x0443, 0x0436, 0x0432, 0x044C, 0x044B, 0x0437, 0x0448, 0x044D, 0x0449, 0x0447, 0x044A,
    0x042E, 0x0410, 0x0411, 0x0426, 0x0414, 0x0415, 0x0424, 0x0413, 0x0425, 0x0418, 0x0419, 0x041A,
    0x041B, 0x041C, 0x041D, 0x041E, 0x041F, 0x042F, 0x0420, 0x0421, 0x0422, 0x0423, 0x0416, 0x0412,
    0x042C, 0x042B, 0x0417, 0x0428, 0x042D, 0x0429, 0x0427, 0x042A,
];

/// Windows-1251 bytes as text (FB2 files that declare it).
pub fn decode_cp1251(b: &[u8]) -> String {
    decode_table(b, &CP1251)
}

/// KOI8-R bytes as text (FB2 files that declare it).
pub fn decode_koi8r(b: &[u8]) -> String {
    decode_table(b, &KOI8R)
}

fn decode_table(b: &[u8], high: &[u16; 128]) -> String {
    b.iter()
        .map(|&c| {
            if c < 0x80 {
                c as char
            } else {
                char::from_u32(u32::from(high[(c - 0x80) as usize])).unwrap_or('\u{FFFD}')
            }
        })
        .collect()
}

/// Windows-1252 bytes as text (MOBI books, FB2 files that declare it).
pub fn decode_cp1252(b: &[u8]) -> String {
    b.iter()
        .map(|&c| match c {
            0x80..=0x9F => char::from_u32(CP1252_HIGH[(c - 0x80) as usize] as u32)
                .filter(|_| CP1252_HIGH[(c - 0x80) as usize] != 0)
                .unwrap_or('\u{FFFD}'),
            _ => c as char,
        })
        .collect()
}

fn decode_utf16(b: &[u8], little_endian: bool) -> String {
    let units = b.as_chunks::<2>().0.iter().map(|p| {
        if little_endian {
            u16::from_le_bytes([p[0], p[1]])
        } else {
            u16::from_be_bytes([p[0], p[1]])
        }
    });
    char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect()
}

/// Guesses UTF-16 without a byte-order mark: mostly-ASCII text has a zero
/// in every other byte.
fn utf16_without_bom(b: &[u8]) -> Option<bool> {
    let probe = &b[..b.len().min(512) & !1];
    if probe.len() < 8 {
        return None;
    }
    let pairs = probe.len() / 2;
    let zeros_odd = probe.iter().skip(1).step_by(2).filter(|&&x| x == 0).count();
    let zeros_even = probe.iter().step_by(2).filter(|&&x| x == 0).count();
    if zeros_odd * 10 > pairs * 7 && zeros_even == 0 {
        Some(true)
    } else if zeros_even * 10 > pairs * 7 && zeros_odd == 0 {
        Some(false)
    } else {
        None
    }
}

/// Decodes a text file: UTF-8 (with or without BOM), UTF-16 (BOM or
/// detected), otherwise Windows-1252. Flags Bijoy-encoded Bangla.
pub fn decode_text(bytes: &[u8]) -> DecodedText {
    let (text, encoding) = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        (
            String::from_utf8_lossy(rest).into_owned(),
            TextEncoding::Utf8,
        )
    } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        (decode_utf16(rest, true), TextEncoding::Utf16Le)
    } else if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        (decode_utf16(rest, false), TextEncoding::Utf16Be)
    } else if let Some(le) = utf16_without_bom(bytes) {
        let enc = if le {
            TextEncoding::Utf16Le
        } else {
            TextEncoding::Utf16Be
        };
        (decode_utf16(bytes, le), enc)
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        (s.to_string(), TextEncoding::Utf8)
    } else {
        (decode_cp1252(bytes), TextEncoding::Windows1252)
    };
    if looks_like_bijoy(&text) {
        return DecodedText {
            text: crate::bijoy::bijoy_to_unicode(&text),
            encoding: TextEncoding::LegacyBijoy,
        };
    }
    DecodedText { text, encoding }
}

/// Symbols Bijoy uses for vowel signs, reph, phala and conjuncts. In real
/// English or European text they are rare.
fn is_bijoy_marker(c: char) -> bool {
    matches!(
        c,
        '†' | '‡'
            | 'ˆ'
            | 'Š'
            | '©'
            | 'ª'
            | '¨'
            | '¯'
            | '¸'
            | '¼'
            | '½'
            | '¾'
            | '×'
            | '‰'
            | '¦'
            | 'Ø'
            | 'Ù'
            | 'Ú'
            | 'Û'
            | 'Ü'
            | 'Ý'
            | 'Þ'
            | 'ß'
            | '¤'
            | '¢'
            | '£'
            | '¥'
            | 'œ'
            | 'Œ'
    )
}

/// Very common Bangla words as Bijoy writes them (আমি, এবং, করা, হয়, না,
/// আর, এই, তার, তিনি, আমার, আমরা, করে, হয়ে, থেকে, করতে, হবে, যে, কি).
const BIJOY_WORDS: &[&str] = &[
    "Avwg", "Ges", "Kiv", "nq", "bv", "Avi", "GB", "Zvi", "wZwb", "Avgvi", "Avgiv", "K‡i", "n‡q",
    "†_‡K", "Ki‡Z", "n‡e", "‡h", "wK", "nq|", "bv|", "Kiv|",
];

const ENGLISH_WORDS: &[&str] = &[
    "the", "and", "of", "to", "is", "in", "that", "it", "was", "for", "on", "with", "as", "a",
];

/// Whether `text` is Bijoy-encoded Bangla.
pub fn looks_like_bijoy(text: &str) -> bool {
    // Unicode Bangla is not Bijoy.
    if text
        .chars()
        .take(20_000)
        .any(|c| ('\u{0980}'..='\u{09FF}').contains(&c))
    {
        return false;
    }
    let sample: String = text.chars().take(20_000).collect();
    let chars = sample.chars().filter(|c| !c.is_whitespace()).count().max(1);
    let markers = sample.chars().filter(|&c| is_bijoy_marker(c)).count();
    let (mut bijoy, mut english) = (0, 0);
    for w in sample.split_whitespace() {
        let w = w.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '"' | '\'' | '(' | ')'));
        if BIJOY_WORDS.contains(&w) {
            bijoy += 1;
        } else if ENGLISH_WORDS.contains(&w.to_ascii_lowercase().as_str()) {
            english += 1;
        }
    }
    let marker_rate = markers as f32 / chars as f32;
    (marker_rate > 0.01 && bijoy >= 1 && english * 3 <= bijoy * 4)
        || (bijoy >= 3 && english < bijoy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp1252(s: &str) -> Vec<u8> {
        s.chars()
            .map(|c| {
                let u = c as u32;
                if u < 0x80 || (0xA0..=0xFF).contains(&u) {
                    u as u8
                } else {
                    0x80 + CP1252_HIGH
                        .iter()
                        .position(|&x| x as u32 == u)
                        .expect("in cp1252") as u8
                }
            })
            .collect()
    }

    #[test]
    fn decodes_common_encodings() {
        let s = "আমার সোনার বাংলা";
        assert_eq!(decode_text(s.as_bytes()).encoding, TextEncoding::Utf8);
        let mut bom = b"\xEF\xBB\xBF".to_vec();
        bom.extend_from_slice(s.as_bytes());
        assert_eq!(decode_text(&bom).text, s);

        let mut le = vec![0xFF, 0xFE];
        le.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let d = decode_text(&le);
        assert_eq!((d.text.as_str(), d.encoding), (s, TextEncoding::Utf16Le));

        let mut be = vec![0xFE, 0xFF];
        be.extend(s.encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(decode_text(&be).text, s);

        let no_bom: Vec<u8> = "Hello reader, welcome"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode_text(&no_bom).text, "Hello reader, welcome");

        let western = "Café — naïve résumé, “quoted” and the end";
        let d = decode_text(&cp1252(western));
        assert_eq!(
            (d.text.as_str(), d.encoding),
            (western, TextEncoding::Windows1252)
        );
    }

    #[test]
    fn detects_bijoy() {
        // আমি বাংলায় গান গাই। আমার সোনার বাংলা, আমি তোমায় ভালোবাসি। (in Bijoy)
        let bijoy = "Avwg evsjvq Mvb MvB| Avgvi †mvbvi evsjv, Avwg †Zvgvq fvjevwm| G‡`k Avgvi, Avwg G‡K fvjevwm|";
        let d = decode_text(&cp1252(bijoy));
        assert_eq!(d.encoding, TextEncoding::LegacyBijoy);
        assert!(
            d.text.starts_with("আমি বাংলায় গান গাই। আমার সোনার বাংলা"),
            "{}",
            d.text
        );
        // Same text saved as UTF-8 is still Bijoy.
        assert_eq!(
            decode_text(bijoy.as_bytes()).encoding,
            TextEncoding::LegacyBijoy
        );
        // Ordinary English is not.
        assert!(!looks_like_bijoy(
            "The quick brown fox jumps over the lazy dog. It was a fine day in the park."
        ));
        // Unicode Bangla is not.
        assert!(!looks_like_bijoy("আমি বাংলায় গান গাই"));
    }
}

#[cfg(test)]
mod cyrillic_tests {
    use super::*;

    #[test]
    fn cyrillic_code_pages() {
        assert_eq!(decode_cp1251(b"\xcf\xf0\xe8\xe2\xe5\xf2 \xb9"), "Привет №");
        assert_eq!(decode_koi8r(b"\xf0\xd2\xc9\xd7\xc5\xd4"), "Привет");
    }
}
