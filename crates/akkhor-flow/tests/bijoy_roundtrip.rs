//! Bijoy conversion checked by round trip: Unicode → Bijoy with the
//! `poriborton` crate (an independent implementation), then back with ours.

use akkhor_flow::bijoy::bijoy_to_unicode;

fn corpus(name: &str) -> String {
    let path = format!("{}/../../tests/corpus/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

/// The spelling both sides are compared in: decomposed nukta letters.
fn norm(s: &str) -> String {
    s.replace('\u{09DC}', "\u{09A1}\u{09BC}")
        .replace('\u{09DD}', "\u{09A2}\u{09BC}")
        .replace('\u{09DF}', "\u{09AF}\u{09BC}")
}

fn words() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in ["bn.txt", "bn-conjuncts.txt"] {
        for w in corpus(f).split(|c: char| c.is_whitespace() || ",।:;()-".contains(c)) {
            if !w.is_empty() {
                out.push(norm(w));
            }
        }
    }
    // Every conjunct of the coverage list with every vowel sign, before and
    // after a plain consonant: the hard cases for visual ordering.
    let conjuncts: Vec<String> = corpus("bn-conjuncts.txt")
        .split_whitespace()
        .map(norm)
        .collect();
    for c in &conjuncts {
        for m in ["া", "ি", "ী", "ু", "ূ", "ৃ", "ে", "ৈ", "ো", "ৌ"] {
            out.push(format!("ক{c}{m}"));
            out.push(format!("{c}{m}ন"));
        }
        out.push(format!("র্{c}"));
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn round_trip_through_poriborton() {
    let words = words();
    let mut wrong = Vec::new();
    for w in &words {
        // poriborton expects the composed ড় ঢ় য়.
        let composed = w
            .replace("\u{09A1}\u{09BC}", "\u{09DC}")
            .replace("\u{09A2}\u{09BC}", "\u{09DD}")
            .replace("\u{09AF}\u{09BC}", "\u{09DF}");
        let bijoy = poriborton::bijoy2000::unicode_to_bijoy(&composed);
        let back = norm(&bijoy_to_unicode(&bijoy));
        if back != *w {
            wrong.push((w.clone(), bijoy, back));
        }
    }
    let exact = 1.0 - wrong.len() as f64 / words.len() as f64;
    println!("{} words, {:.2}% exact", words.len(), exact * 100.0);
    for (w, b, back) in wrong.iter().take(40) {
        println!("  {w} -> {b} -> {back}");
    }
    assert!(
        exact >= 0.98,
        "round trip {:.2}% exact, target 98%",
        exact * 100.0
    );
}
