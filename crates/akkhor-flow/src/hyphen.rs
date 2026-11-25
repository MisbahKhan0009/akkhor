//! English hyphenation (Liang's algorithm, TeX patterns).
//!
//! Only Latin words are hyphenated: Bangla and Devanagari typesetting does
//! not hyphenate, and an over-long Indic word is broken between syllables
//! by the layout instead. Patterns are parsed on first use.

use std::collections::HashMap;
use std::sync::OnceLock;

const LEFT_MIN: usize = 2;
const RIGHT_MIN: usize = 3;

struct Patterns {
    table: HashMap<Vec<u8>, Vec<u8>>,
    exceptions: HashMap<String, Vec<usize>>,
    longest: usize,
}

#[cfg(feature = "hyphenation")]
const PATTERNS: &str = include_str!("../data/hyph-en-us.pat.txt");
#[cfg(feature = "hyphenation")]
const EXCEPTIONS: &str = include_str!("../data/hyph-en-us.hyp.txt");
#[cfg(not(feature = "hyphenation"))]
const PATTERNS: &str = "";
#[cfg(not(feature = "hyphenation"))]
const EXCEPTIONS: &str = "";

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let mut table = HashMap::new();
        let mut longest = 0;
        for pat in PATTERNS.split_whitespace() {
            let mut letters = Vec::new();
            let mut values = vec![0u8];
            for b in pat.bytes() {
                if b.is_ascii_digit() {
                    *values.last_mut().expect("non-empty") = b - b'0';
                } else {
                    letters.push(b);
                    values.push(0);
                }
            }
            longest = longest.max(letters.len());
            table.insert(letters, values);
        }
        let mut exceptions = HashMap::new();
        for word in EXCEPTIONS.split_whitespace() {
            let mut points = Vec::new();
            let mut plain = String::new();
            for c in word.chars() {
                if c == '-' {
                    points.push(plain.len());
                } else {
                    plain.push(c);
                }
            }
            exceptions.insert(plain, points);
        }
        Patterns {
            table,
            exceptions,
            longest,
        }
    })
}

/// Byte offsets inside `word` where it may be hyphenated. Returns nothing
/// for words that are not plain ASCII letters (apart from a trailing
/// apostrophe-s), or that are too short.
pub fn hyphenate(word: &str) -> Vec<usize> {
    if word.len() < LEFT_MIN + RIGHT_MIN || !word.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let p = patterns();
    let lower = word.to_ascii_lowercase();
    if let Some(points) = p.exceptions.get(&lower) {
        return points.clone();
    }
    let mut w = Vec::with_capacity(lower.len() + 2);
    w.push(b'.');
    w.extend_from_slice(lower.as_bytes());
    w.push(b'.');
    let mut values = vec![0u8; w.len() + 1];
    for i in 0..w.len() {
        for j in i + 1..=(i + p.longest).min(w.len()) {
            if let Some(pat) = p.table.get(&w[i..j]) {
                for (k, &v) in pat.iter().enumerate() {
                    values[i + k] = values[i + k].max(v);
                }
            }
        }
    }
    // values[c + 1] is the digit before word[c] (w has a leading dot).
    (LEFT_MIN..=word.len() - RIGHT_MIN)
        .filter(|&c| values[c + 1] % 2 == 1)
        .collect()
}

#[cfg(all(test, feature = "hyphenation"))]
mod tests {
    use super::*;

    fn show(word: &str) -> String {
        let mut out = String::new();
        let points = hyphenate(word);
        for (i, c) in word.char_indices() {
            if points.contains(&i) {
                out.push('-');
            }
            out.push(c);
        }
        out
    }

    #[test]
    fn hyphenates_english() {
        assert_eq!(show("hyphenation"), "hy-phen-ation");
        assert_eq!(show("associate"), "as-so-ciate");
        assert_eq!(show("table"), "ta-ble");
        assert_eq!(show("computer"), "com-puter"); // "er" is shorter than the 3-letter minimum
        assert_eq!(show("Reading"), "Read-ing");
    }

    #[test]
    fn leaves_other_words_alone() {
        assert!(hyphenate("cat").is_empty());
        assert!(hyphenate("বাংলাদেশ").is_empty());
        assert!(hyphenate("e-mail").is_empty());
    }
}
