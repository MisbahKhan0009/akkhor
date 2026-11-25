//! Words on the book-layout corpus (tests/corpus/books): every word of the
//! ground truth must come out of the page text whole, not run into its
//! neighbour or split by letters the producer repeated. Line-end hyphens are
//! joined first: they are a reflow question (Phase 4.3), not a text-layer
//! one.

mod common;

use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use common::*;

/// The `"text"` values of a truth file. The files are written by
/// scripts/make_books.py with `ensure_ascii=False`; the texts hold no
/// quotes or backslashes, so a plain scan is enough.
fn truth_texts(json: &str) -> Vec<String> {
    json.split("\"text\": \"")
        .skip(1)
        .filter_map(|s| s.split('"').next().map(str::to_string))
        .collect()
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || ",।॥:;()-!?\"“”‘’'·.".contains(c)
}

fn tokens(s: &str) -> Vec<String> {
    s.split(is_separator)
        .filter(|w| !w.is_empty())
        .map(|w| akkhor_flow::search::fold(w).text)
        .collect()
}

/// Joins `word-⏎word` across line ends.
fn join_hyphens(s: &str) -> String {
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

#[test]
fn book_words_come_out_whole() {
    register_test_fonts();
    let dir = corpus("books/pdf");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pdf"))
        .collect();
    names.sort();
    assert!(names.len() >= 20, "book corpus missing: {}", names.len());
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
        let target = if name.contains("mukta") { 0.99 } else { 1.0 };
        if intact < target {
            failed.push(format!("{name}: {:.1}%", intact * 100.0));
        }
    }
    assert!(failed.is_empty(), "words not intact: {failed:?}");
}
