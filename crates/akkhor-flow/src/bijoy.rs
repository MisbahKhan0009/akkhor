//! Bijoy to Unicode.
//!
//! Bijoy (SutonnyMJ and the other "MJ" fonts) is how most Bangla was typed
//! before Unicode, and much of it still is: the file holds Latin-1 bytes
//! that only look like Bangla in a Bijoy font. It is written in visual
//! order: ি ে ৈ come before their consonant, the reph after it, and ো is
//! ে … া around the consonant.
//!
//! Conversion: the text is cut into Bijoy pieces by longest match against a
//! table (see `bijoy_table.rs`, derived from the MIT licensed `poriborton`
//! crate), each piece becomes Unicode, and pieces are put into logical
//! order.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::bijoy_table::TABLE;
use crate::script::{IndicCat, indic_cat};

struct Table {
    map: HashMap<&'static str, &'static str>,
    longest: usize,
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| Table {
        map: TABLE.iter().copied().collect(),
        longest: TABLE
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(1),
    })
}

/// Converts Bijoy text (decoded as Windows-1252) to Unicode Bangla.
/// Characters outside the Bijoy table (spaces, Latin punctuation) pass
/// through.
pub fn bijoy_to_unicode(input: &str) -> String {
    let t = table();
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut out = String::with_capacity(input.len() * 2);
    let mut word: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = chars[i].0;
        let hit = (1..=t.longest.min(chars.len() - i)).rev().find_map(|n| {
            let end = chars.get(i + n).map_or(input.len(), |c| c.0);
            t.map.get(&input[start..end]).map(|u| (n, *u))
        });
        match hit {
            Some((n, u)) => {
                word.push(u.to_string());
                i += n;
            }
            None => {
                flush(&mut word, &mut out);
                out.push(chars[i].1);
                i += 1;
            }
        }
    }
    flush(&mut word, &mut out);
    out
}

fn flush(word: &mut Vec<String>, out: &mut String) {
    if !word.is_empty() {
        out.push_str(&reorder(std::mem::take(word)));
    }
}

fn first_cat(s: &str) -> Option<IndicCat> {
    s.chars().next().and_then(indic_cat).map(|(_, k)| k)
}

fn starts_with_letter(s: &str) -> bool {
    matches!(first_cat(s), Some(IndicCat::Consonant | IndicCat::Vowel))
}

fn ends_with_virama(s: &str) -> bool {
    s.ends_with('\u{09CD}')
}

fn continues_cluster(s: &str) -> bool {
    matches!(first_cat(s), Some(IndicCat::Virama | IndicCat::Nukta))
}

fn is_prebase(s: &str) -> bool {
    matches!(s, "\u{09BF}" | "\u{09C7}" | "\u{09C8}")
}

/// Puts one run of Bijoy pieces into logical order.
fn reorder(pieces: Vec<String>) -> String {
    // 1. Pre-base vowel signs after their consonant cluster.
    let mut out: Vec<String> = Vec::with_capacity(pieces.len());
    let mut pending: Vec<String> = Vec::new();
    let mut it = pieces.into_iter().peekable();
    while let Some(p) = it.next() {
        if is_prebase(&p) {
            pending.push(p);
            continue;
        }
        let letter = starts_with_letter(&p);
        out.push(p);
        if letter && !pending.is_empty() {
            while let Some(next) = it.peek() {
                let joined = out.last().is_some_and(|q| ends_with_virama(q));
                if !is_prebase(next) && (joined || continues_cluster(next)) {
                    out.push(it.next().expect("peeked"));
                } else {
                    break;
                }
            }
            out.append(&mut pending);
        }
    }
    out.append(&mut pending);

    // 2. Reph (written after its consonant cluster, and after a pre-base
    //    vowel sign that was moved behind it) to the cluster's start.
    let mut i = 0;
    while i < out.len() {
        if out[i] == "\u{09B0}\u{09CD}" && i > 0 {
            let mut j = i;
            while j > 0 {
                j -= 1;
                if starts_with_letter(&out[j]) && !(j > 0 && ends_with_virama(&out[j - 1])) {
                    break;
                }
            }
            if j < i && starts_with_letter(&out[j]) {
                let reph = out.remove(i);
                out.insert(j, reph);
            }
        }
        i += 1;
    }

    // 3. Text: two-part vowels joined, র + ্য kept apart with a ZWJ (র‍্য,
    //    not a reph over য).
    let mut text = String::new();
    for p in out {
        if text.ends_with('\u{09C7}') && (p.starts_with('\u{09BE}') || p.starts_with('\u{09D7}')) {
            text.pop();
            text.push(if p.starts_with('\u{09BE}') {
                '\u{09CB}'
            } else {
                '\u{09CC}'
            });
            text.push_str(&p[3..]);
            continue;
        }
        // A half form (ন্) before a below-base form (্তু): one virama.
        if text.ends_with('\u{09CD}') && p.starts_with('\u{09CD}') {
            text.pop();
        }
        if text.ends_with('\u{09B0}') && p.starts_with("\u{09CD}\u{09AF}") {
            text.push('\u{200D}');
        }
        text.push_str(&p);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_words() {
        let cases = [
            ("Avwg evsjvq Mvb MvB", "আমি বাংলায় গান গাই"),
            ("weRq 2000", "বিজয় ২০০০"),
            ("†Kv", "কো"),
            ("†KŠ", "কৌ"),
            ("AK©", "অর্ক"),
            ("wKsKZ©e¨weg~p", "কিংকর্তব্যবিমূঢ়"),
            ("†¯cK", "স্পেক"),
            ("we`¨yr", "বিদ্যুৎ"),
            ("wKš‘", "কিন্তু"),
            ("i¨ve", "র\u{200D}্যাব"),
            ("Pvu`", "চাঁদ"),
            ("`ytL", "দুঃখ"),
        ];
        for (bijoy, unicode) in cases {
            let want = unicode.replace('\u{09DF}', "\u{09AF}\u{09BC}");
            assert_eq!(bijoy_to_unicode(bijoy), want, "{bijoy}");
        }
    }

    #[test]
    fn leaves_other_text_alone() {
        assert_eq!(bijoy_to_unicode("(1) Avwg!"), "(১) আমি!");
        assert_eq!(bijoy_to_unicode(""), "");
    }
}
