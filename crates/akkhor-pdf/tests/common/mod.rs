//! Shared corpus runner for the accuracy tests.

#![allow(dead_code)]

use std::sync::Arc;

use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

pub fn corpus(name: &str) -> String {
    format!("{}/../../tests/corpus/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Folded, whitespace-free characters: what a reader would compare.
pub fn normal(s: &str) -> Vec<char> {
    akkhor_flow::search::fold(s)
        .text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + (ca != cb) as usize)
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn cer(truth: &str, got: &str) -> f64 {
    let (t, g) = (normal(truth), normal(got));
    edit_distance(&t, &g) as f64 / t.len().max(1) as f64
}

pub fn document_text(pdf: &PdfDocument, raw: bool) -> String {
    (0..pdf.page_count())
        .map(|i| {
            if raw {
                pdf.page_text_raw(i)
            } else {
                pdf.page_text(i)
            }
            .unwrap()
            .text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Registers the corpus fonts as reference fonts: the extra designs in
/// tests/fonts, and the Noto ones (also compiled in with `bundled-fonts`).
pub fn register_test_fonts() {
    for f in [
        "../fonts/HindSiliguri-Regular.ttf",
        "../fonts/Mukta-Regular.ttf",
        "../fonts/Kalpurush-Regular.ttf",
        "../../fonts/NotoSerifBengali-Regular.ttf",
        "../../fonts/NotoSerifDevanagari-Regular.ttf",
        "../../fonts/NotoSerif-Regular.ttf",
    ] {
        let data = std::fs::read(corpus(f)).unwrap();
        akkhor_pdf::register_reference_font(Arc::new(data));
    }
}

/// One row per corpus PDF: (file, CER of the PDF's own text, CER of
/// Akkhor's text).
pub fn run_corpus() -> Vec<(String, f64, f64)> {
    let mut rows = Vec::new();
    for stem in ["bn", "hi", "bn-conjuncts", "hi-conjuncts"] {
        let truth = std::fs::read_to_string(corpus(&format!("{stem}.txt"))).unwrap();
        let design = if stem.starts_with("bn") {
            "hind"
        } else {
            "mukta"
        };
        for base in [stem.to_string(), format!("{stem}.{design}")] {
            for producer in ["chrome", "libreoffice"] {
                for variant in ["", ".raw"] {
                    let file = format!("pdf/{base}.{producer}{variant}.pdf");
                    let pdf = PdfDocument::open(&Source::open(corpus(&file)).unwrap()).unwrap();
                    let before = cer(&truth, &document_text(&pdf, true));
                    let after = cer(&truth, &document_text(&pdf, false));
                    rows.push((file, before, after));
                }
            }
        }
    }
    // Set D: bn.txt typed in Bijoy, in a font named SutonnyMJ.
    let truth = std::fs::read_to_string(corpus("bn.txt")).unwrap();
    for producer in ["chrome", "libreoffice"] {
        for variant in ["", ".raw"] {
            let file = format!("pdf/bn.bijoy.{producer}{variant}.pdf");
            let pdf = PdfDocument::open(&Source::open(corpus(&file)).unwrap()).unwrap();
            let before = cer(&truth, &document_text(&pdf, true));
            let after = cer(&truth, &document_text(&pdf, false));
            rows.push((file, before, after));
        }
    }
    println!("{:<48} {:>10} {:>10}", "file", "per glyph", "Akkhor");
    for (f, b, a) in &rows {
        println!("{f:<48} {:>9.1}% {:>9.1}%", b * 100.0, a * 100.0);
    }
    rows
}

/// Targets (docs/phases/phase-2.md): under 2% with the PDF's text layer,
/// under 5% without it.
pub fn assert_targets(rows: &[(String, f64, f64)]) {
    for (f, _, a) in rows {
        let limit = if f.contains(".raw") { 0.05 } else { 0.02 };
        assert!(
            *a < limit,
            "{f}: {:.1}% is over {:.0}%",
            a * 100.0,
            limit * 100.0
        );
    }
}

// --- book corpus (tests/corpus/books) ---

/// The `"text"` values of a truth file. The files are written by
/// scripts/make_books.py with `ensure_ascii=False`; the texts hold no
/// quotes or backslashes, so a plain scan is enough.
pub fn truth_texts(json: &str) -> Vec<String> {
    json.split("\"text\": \"")
        .skip(1)
        .filter_map(|s| s.split('"').next().map(str::to_string))
        .collect()
}

pub fn is_separator(c: char) -> bool {
    c.is_whitespace() || ",।॥:;()-!?\"“”‘’'·.".contains(c)
}

pub fn tokens(s: &str) -> Vec<String> {
    s.split(is_separator)
        .filter(|w| !w.is_empty())
        .map(|w| akkhor_flow::search::fold(w).text)
        .collect()
}

/// Joins `word-⏎word` across line ends.
pub fn join_hyphens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '-' | '\u{2010}' | '\u{ad}') && chars.peek() == Some(&'\n') {
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// Every book PDF `pick` selects must keep `target(name)` of its words whole.
pub fn check_books(pick: impl Fn(&str) -> bool, target: impl Fn(&str) -> f64) {
    let dir = corpus("books/pdf");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pdf") && pick(n))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "book corpus missing");
    let mut failed = Vec::new();
    for name in &names {
        let path = format!("{dir}/{name}");
        let truth = std::fs::read_to_string(path.replace(".pdf", ".json")).unwrap();
        let words: Vec<String> = truth_texts(&truth)
            .iter()
            .flat_map(|t| tokens(t))
            .filter(|w| w.chars().count() >= 2)
            .collect();
        let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
        let text: String = (0..doc.page_count())
            .map(|i| doc.page_text(i).unwrap().text)
            .collect::<Vec<_>>()
            .join("\n");
        // Joined and as printed: a line-end hyphen may be a real compound
        // (घर-घर), which reflow must keep.
        let have: std::collections::HashSet<String> = tokens(&join_hyphens(&text))
            .into_iter()
            .chain(tokens(&text))
            .collect();
        let missing: Vec<&String> = words.iter().filter(|w| !have.contains(*w)).collect();
        let intact = 1.0 - missing.len() as f64 / words.len() as f64;
        println!(
            "{name:34} {:5.1}% intact  {:?}",
            intact * 100.0,
            &missing[..missing.len().min(4)]
        );
        // Mukta's nukta and candrabindu glyphs are known to read wrong in
        // XeLaTeX output (see docs/reviews/phase-4.md); everything else must
        // be whole.
        if intact < target(name) {
            failed.push(format!("{name}: {:.1}%", intact * 100.0));
        }
    }
    assert!(failed.is_empty(), "words not intact: {failed:?}");
}
