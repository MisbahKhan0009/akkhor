//! Whole-book search (Phase 5.3): hits in book order with snippets, handed
//! out a few at a time, PDF hits as page places.

use akkhor::search::BookSearch;
use akkhor::{Book, Locator, Reading};

fn path(rel: &str) -> String {
    format!("{}/../../tests/{rel}", env!("CARGO_MANIFEST_DIR"))
}

/// Text from `start` to `end` of one block.
fn hit_text(r: &mut Reading, book: &Book, start: Locator, end: Locator) -> String {
    match (start, end) {
        (
            Locator::Flow {
                chapter,
                block,
                offset,
            },
            Locator::Flow { offset: e, .. },
        ) => r.chapter(book, chapter).unwrap().document.blocks[block].text[offset..e].to_string(),
        (Locator::Page { page, offset }, Locator::Page { offset: e, .. }) => {
            let Book::Pdf(p) = book else { unreachable!() };
            p.page_text(page).unwrap().text[offset..e].to_string()
        }
        _ => panic!("mixed locators"),
    }
}

#[test]
fn epub_hits_come_in_order_a_few_at_a_time() {
    let book = akkhor::open(path("fixtures/indic-sample.epub")).unwrap();
    let mut r = Reading::new(&book);
    let mut s = BookSearch::new("প্রতিটি");
    let mut all = Vec::new();
    loop {
        let b = s.next(&book, &mut r, 1);
        assert!(b.hits.len() <= 1);
        all.extend(b.hits);
        if b.done {
            assert_eq!(b.searched, 1.0);
            break;
        }
    }
    assert!(all.len() >= 3, "{}", all.len());
    for w in all.windows(2) {
        assert!(w[0].start < w[1].start, "out of order");
    }
    for h in &all {
        assert_eq!(hit_text(&mut r, &book, h.start, h.end), "প্রতিটি");
        let units: Vec<u16> = h.snippet.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[h.snippet_match.clone()]).unwrap(),
            "প্রতিটি"
        );
    }
    // The batch as JSON for the app.
    let mut s = BookSearch::new("অক্ষর");
    let json = s.next(&book, &mut r, 10).to_json();
    assert!(
        json.starts_with(r#"{"hits":[{"start":"akkhor:1/c0/"#),
        "{json}"
    );
}

#[test]
fn pdf_hits_are_page_places_and_find_hyphenated_words() {
    let book = akkhor::open(path("corpus/books/pdf/hi.chrome.two.pdf")).unwrap();
    let mut r = Reading::new(&book);
    // धीरे-धीरे is split by a line-end hyphen in this file.
    let mut s = BookSearch::new("धीरे-धीरे");
    let mut hits = Vec::new();
    loop {
        let b = s.next(&book, &mut r, 5);
        hits.extend(b.hits);
        if b.done {
            break;
        }
    }
    assert!(!hits.is_empty());
    assert!(matches!(hits[0].start, Locator::Page { .. }));
    // In the reflow view the same hit is a reflow place.
    let flow = r.convert(&book, hits[0].start).unwrap();
    assert!(matches!(flow, Locator::Flow { .. }));
}

#[test]
fn nothing_found_and_empty_queries_finish() {
    let book = akkhor::open(path("corpus/bn.txt")).unwrap();
    let mut r = Reading::new(&book);
    for q in ["zzzz-not-there", "", "   "] {
        let b = BookSearch::new(q).next(&book, &mut r, 10);
        assert!(b.hits.is_empty() && b.done, "{q:?}");
    }
}
