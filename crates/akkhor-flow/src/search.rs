//! Indic-aware search.
//!
//! The same word reaches books in different but equivalent encodings: a
//! nukta letter as one code point or as base + nukta, Bangla ো as one code
//! point or as two, khanda ta as ৎ or as ত্‍, with or without ZWJ/ZWNJ.
//! Search folds both the text and the query to one form, keeps a map back to
//! the original offsets, and refuses matches that would cut a conjunct: "ক"
//! finds কি and কলম but not the first half of ক্ষ.

use crate::model::{Document, Position};
use crate::script::{IndicCat, indic_cat, is_syllable_boundary};

/// Text folded for matching, with a map from every folded byte (and the end)
/// back to the original byte offset.
#[derive(Debug, Clone)]
pub struct Folded {
    pub text: String,
    pub origin: Vec<usize>,
}

/// Canonical decompositions that matter for Indic matching.
fn decompose(c: char) -> Option<&'static str> {
    Some(match c {
        // Devanagari nukta letters
        '\u{0929}' => "\u{0928}\u{093C}",
        '\u{0931}' => "\u{0930}\u{093C}",
        '\u{0934}' => "\u{0933}\u{093C}",
        '\u{0958}' => "\u{0915}\u{093C}",
        '\u{0959}' => "\u{0916}\u{093C}",
        '\u{095A}' => "\u{0917}\u{093C}",
        '\u{095B}' => "\u{091C}\u{093C}",
        '\u{095C}' => "\u{0921}\u{093C}",
        '\u{095D}' => "\u{0922}\u{093C}",
        '\u{095E}' => "\u{092B}\u{093C}",
        '\u{095F}' => "\u{092F}\u{093C}",
        // Bengali: nukta letters, split vowels, khanda ta
        '\u{09DC}' => "\u{09A1}\u{09BC}",
        '\u{09DD}' => "\u{09A2}\u{09BC}",
        '\u{09DF}' => "\u{09AF}\u{09BC}",
        '\u{09CB}' => "\u{09C7}\u{09BE}",
        '\u{09CC}' => "\u{09C7}\u{09D7}",
        '\u{09CE}' => "\u{09A4}\u{09CD}",
        // Gurmukhi nukta letters
        '\u{0A33}' => "\u{0A32}\u{0A3C}",
        '\u{0A36}' => "\u{0A38}\u{0A3C}",
        '\u{0A59}' => "\u{0A16}\u{0A3C}",
        '\u{0A5A}' => "\u{0A17}\u{0A3C}",
        '\u{0A5B}' => "\u{0A1C}\u{0A3C}",
        '\u{0A5E}' => "\u{0A2B}\u{0A3C}",
        // Odia
        '\u{0B5C}' => "\u{0B21}\u{0B3C}",
        '\u{0B5D}' => "\u{0B22}\u{0B3C}",
        '\u{0B48}' => "\u{0B47}\u{0B56}",
        '\u{0B4B}' => "\u{0B47}\u{0B3E}",
        '\u{0B4C}' => "\u{0B47}\u{0B57}",
        // Tamil, Telugu, Kannada, Malayalam split vowels
        '\u{0BCA}' => "\u{0BC6}\u{0BBE}",
        '\u{0BCB}' => "\u{0BC7}\u{0BBE}",
        '\u{0BCC}' => "\u{0BC6}\u{0BD7}",
        '\u{0C48}' => "\u{0C46}\u{0C56}",
        '\u{0CC0}' => "\u{0CBF}\u{0CD5}",
        '\u{0CC7}' => "\u{0CC6}\u{0CD5}",
        '\u{0CC8}' => "\u{0CC6}\u{0CD6}",
        '\u{0CCA}' => "\u{0CC6}\u{0CC2}",
        '\u{0CCB}' => "\u{0CC6}\u{0CC2}\u{0CD5}",
        '\u{0D4A}' => "\u{0D46}\u{0D3E}",
        '\u{0D4B}' => "\u{0D47}\u{0D3E}",
        '\u{0D4C}' => "\u{0D46}\u{0D57}",
        // Typographic quotes
        '\u{2018}' | '\u{2019}' | '\u{201B}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201F}' => "\"",
        _ => return None,
    })
}

/// Folds text for matching.
pub fn fold(text: &str) -> Folded {
    let mut out = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len() + 1);
    let push = |out: &mut String, origin: &mut Vec<usize>, c: char, at: usize| {
        let before = out.len();
        out.push(c);
        origin.extend(std::iter::repeat_n(at, out.len() - before));
    };
    for (at, c) in text.char_indices() {
        match c {
            '\u{200C}' | '\u{200D}' | '\u{00AD}' | '\u{FEFF}' => {}
            c if c.is_whitespace() => {
                if !out.is_empty() && !out.ends_with(' ') {
                    push(&mut out, &mut origin, ' ', at);
                }
            }
            c => {
                if let Some(d) = decompose(c) {
                    for dc in d.chars() {
                        push(&mut out, &mut origin, dc, at);
                    }
                } else if c.is_uppercase() {
                    for lc in c.to_lowercase() {
                        push(&mut out, &mut origin, lc, at);
                    }
                } else {
                    push(&mut out, &mut origin, c, at);
                }
            }
        }
    }
    origin.push(text.len());
    Folded { text: out, origin }
}

/// One search hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: Position,
    pub end: Position,
}

fn joins_next(c: char) -> bool {
    matches!(indic_cat(c), Some((_, IndicCat::Virama | IndicCat::Nukta)))
}

/// Whether a match may start at byte `i` of folded text: on a syllable start.
fn may_start(t: &str, i: usize) -> bool {
    let mut before = t[..i].chars().rev();
    let (Some(prev), Some(cur)) = (before.next(), t[i..].chars().next()) else {
        return true;
    };
    is_syllable_boundary(before.next(), prev, cur)
}

/// Whether a match may end at byte `i` of folded text: not before a mark
/// that would join it to the next letter, and not after a virama that
/// joins the next consonant.
fn may_end(t: &str, i: usize) -> bool {
    let next = t[i..].chars().next();
    let mut before = t[..i].chars().rev();
    let last = before.next();
    match (last, next) {
        (_, Some(n)) if joins_next(n) => false,
        (Some(l), Some(n)) => is_syllable_boundary(before.next(), l, n) || !is_conjunct_join(l),
        _ => true,
    }
}

/// A virama in the middle of a conjunct: ending a match right after it would
/// split the conjunct.
fn is_conjunct_join(c: char) -> bool {
    matches!(indic_cat(c), Some((_, IndicCat::Virama)))
}

/// Finds every occurrence of `query` in one block of text, as byte ranges.
pub fn find_in_text(text: &str, query: &Folded) -> Vec<(usize, usize)> {
    let q = query.text.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let f = fold(text);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = f.text[from..].find(q) {
        let s = from + rel;
        let e = s + q.len();
        if may_start(&f.text, s) && may_end(&f.text, e) {
            let (os, oe) = (f.origin[s], f.origin[e]);
            if oe > os {
                out.push((os, oe));
            }
        }
        from = s + f.text[s..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Finds every occurrence of `query` in a document.
pub fn find_all(doc: &Document, query: &str) -> Vec<Match> {
    let q = fold(query);
    let mut out = Vec::new();
    for (b, block) in doc.blocks.iter().enumerate() {
        for (s, e) in find_in_text(&block.text, &q) {
            out.push(Match {
                start: Position::new(b, s),
                end: Position::new(b, e),
            });
        }
    }
    out
}

/// A short piece of text around a match, for a results list: the text and
/// the byte range of the match inside it.
pub fn snippet(
    doc: &Document,
    m: &Match,
    context_chars: usize,
) -> (String, std::ops::Range<usize>) {
    let Some(block) = doc.blocks.get(m.start.block) else {
        return (String::new(), 0..0);
    };
    let t = &block.text;
    let end = if m.end.block == m.start.block {
        m.end.offset
    } else {
        t.len()
    };
    let from = t[..m.start.offset]
        .char_indices()
        .rev()
        .nth(context_chars.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    let to = t[end..]
        .char_indices()
        .nth(context_chars)
        .map_or(t.len(), |(i, _)| end + i);
    (t[from..to].to_string(), m.start.offset - from..end - from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(text: &str, query: &str) -> Vec<String> {
        find_in_text(text, &fold(query))
            .into_iter()
            .map(|(s, e)| text[s..e].to_string())
            .collect()
    }

    #[test]
    fn nukta_letters_match_both_ways() {
        // precomposed in text, decomposed in query and the other way round
        assert_eq!(hits("ज़रा सी बात", "ज\u{093C}रा"), vec!["ज़रा"]);
        assert_eq!(hits("ज\u{093C}रा सी", "ज़रा"), vec!["ज\u{093C}रा"]);
        assert_eq!(hits("ব\u{09DC} কথা", "বড\u{09BC}"), vec!["ব\u{09DC}"]);
        assert_eq!(hits("আমি যাই", "আমি"), vec!["আমি"]);
    }

    #[test]
    fn split_vowels_and_khanda_ta() {
        assert_eq!(hits("কোথায়", "ক\u{09C7}\u{09BE}থায়"), vec!["কোথায়"]);
        // ৎ written three ways
        let q = "উৎসব";
        assert_eq!(hits("উৎসব", q).len(), 1);
        assert_eq!(hits("উত্\u{200D}সব", q).len(), 1);
        assert_eq!(hits("উত্সব", q).len(), 1);
    }

    #[test]
    fn joiners_and_case_are_ignored() {
        assert_eq!(hits("ক্\u{200C}ষ", "ক্ষ").len(), 1);
        assert_eq!(hits("The Quick Fox", "quick").len(), 1);
        assert_eq!(hits("it’s here", "it's").len(), 1);
    }

    #[test]
    fn never_matches_half_a_conjunct() {
        // ক alone is in কলম and কি, but not in ক্ষমা
        assert_eq!(hits("ক্ষমা কলম কি", "ক"), vec!["ক", "ক"]);
        // द alone does not match inside द्ध
        assert_eq!(hits("शुद्ध", "द").len(), 0);
        // a match may not start inside a conjunct either
        assert_eq!(hits("ক্ষমা", "ষমা").len(), 0);
        // but the whole conjunct is found
        assert_eq!(hits("ক্ষমা", "ক্ষ").len(), 1);
    }

    #[test]
    fn offsets_map_back_exactly() {
        let doc = Document::from_plain_text("আমার সোনার বাংলা, আমি তোমায় ভালোবাসি।");
        let m = find_all(&doc, "তোমায়");
        assert_eq!(m.len(), 1);
        let t = &doc.blocks[0].text;
        assert_eq!(&t[m[0].start.offset..m[0].end.offset], "তোমায়");
        let (snip, r) = snippet(&doc, &m[0], 5);
        assert_eq!(&snip[r], "তোমায়");
    }
}
