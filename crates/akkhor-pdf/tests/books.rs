//! Words on the book-layout corpus (tests/corpus/books): every word of the
//! ground truth must come out of the page text whole, not run into its
//! neighbour or split by letters the producer repeated. Line-end hyphens are
//! joined first: they are a reflow question (Phase 4.3), not a text-layer
//! one.

mod common;

use common::*;

#[test]
fn book_words_come_out_whole() {
    register_test_fonts();
    check_books(
        |_| true,
        // Mukta draws ब्ज़ and ब्ज with the same glyph, so the one nukta
        // conjunct in the Hindi text (सब्ज़ियाँ) cannot be told apart.
        |name| if name.contains("mukta") { 0.997 } else { 1.0 },
    );
}

/// Page search finds every word of the truth, also the ones split by a
/// line-end hyphen (a word split across two pages would be out of a page
/// search's reach; the corpus has none).
#[test]
fn page_search_finds_hyphenated_words() {
    use akkhor_core::Source;
    use akkhor_pdf::PdfDocument;
    register_test_fonts();
    let dir = corpus("books/pdf");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pdf") && !n.contains("mukta"))
        .collect();
    names.sort();
    let mut failed = Vec::new();
    for name in &names {
        let path = format!("{dir}/{name}");
        let truth = std::fs::read_to_string(path.replace(".pdf", ".json")).unwrap();
        let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
        let pages: Vec<akkhor_pdf::PageText> = (0..doc.page_count())
            .map(|i| doc.page_text(i).unwrap())
            .collect();
        let mut words: Vec<String> = truth_texts(&truth)
            .iter()
            .flat_map(|t| t.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .map(|w| {
                w.trim_matches(|c: char| {
                    !c.is_alphanumeric() && akkhor_flow::script::indic_cat(c).is_none()
                })
                .to_string()
            })
            .filter(|w| w.chars().count() >= 2)
            .collect();
        words.sort();
        words.dedup();
        let missing: Vec<&String> = words
            .iter()
            .filter(|w| pages.iter().all(|p| p.find(w).is_empty()))
            .collect();
        println!(
            "{name:34} {} of {} found {:?}",
            words.len() - missing.len(),
            words.len(),
            &missing[..missing.len().min(3)]
        );
        if !missing.is_empty() {
            failed.push(name.clone());
        }
    }
    assert!(failed.is_empty(), "search misses words: {failed:?}");
}

/// With the corpus fonts registered, every book's text is sure.
#[test]
fn text_quality_is_full_with_the_fonts() {
    use akkhor_core::Source;
    use akkhor_pdf::PdfDocument;
    register_test_fonts();
    let dir = corpus("books/pdf");
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".pdf") {
            continue;
        }
        let doc = PdfDocument::open(&Source::open(e.path()).unwrap()).unwrap();
        let q = doc.text_quality(0..3).unwrap();
        assert!(q >= akkhor_pdf::TEXT_QUALITY_GOOD, "{name}: {q}");
    }
}
