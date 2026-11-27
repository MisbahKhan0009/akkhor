//! Book info and covers for the library (Phase 5.1).

use std::time::Instant;

fn path(rel: &str) -> String {
    format!("{}/../../tests/{rel}", env!("CARGO_MANIFEST_DIR"))
}

fn info(rel: &str) -> akkhor::BookInfo {
    akkhor::open(path(rel)).unwrap().info()
}

#[test]
fn epub_info_from_its_metadata() {
    let i = info("fixtures/indic-sample.epub");
    assert_eq!(i.title.as_deref(), Some("অক্ষর: একটি নমুনা বই"));
    assert_eq!(i.authors, ["Akkhor test suite"]);
    assert_eq!(i.language.as_deref(), Some("bn"));
    assert!(!i.language_guessed);
    assert_eq!(i.length, 3);
    assert!(!i.has_cover && i.reflowable);
    let book = akkhor::open(path("fixtures/indic-sample.epub")).unwrap();
    assert!(book.cover(200, 300).is_none());
}

#[test]
fn pdf_info_guesses_title_and_language() {
    // Chrome writes the HTML file name as the title: dropped, and the
    // largest line of the first page used instead.
    let i = info("corpus/books/pdf/bn.chrome.one.pdf");
    assert_eq!(i.title.as_deref(), Some("নদীর ধারে একটি গ্রাম"));
    assert_eq!(i.language.as_deref(), Some("bn"));
    assert!(i.language_guessed && i.reflowable && i.has_cover);
    assert_eq!(i.text_quality, Some(1.0));
    assert!(!i.needs_fonts);
    let i = info("corpus/books/pdf/hi.writer.one.pdf");
    assert_eq!(i.language.as_deref(), Some("hi"));
    let json = i.to_json();
    assert!(json.starts_with(r#"{"format":"pdf","#), "{json}");
    assert!(json.contains(r#""language":"hi""#), "{json}");
}

#[test]
fn pdf_title_and_authors_from_xmp() {
    // No information dictionary entries; title and creators only in XMP
    // (tests/fixtures/xmp-only.pdf, made with PyMuPDF).
    let i = info("fixtures/xmp-only.pdf");
    assert_eq!(i.title.as_deref(), Some("নদীর ধারে"));
    assert_eq!(i.authors, ["লেখক এক, Writer Two"]);
}

#[test]
fn text_info_and_bijoy() {
    let i = info("corpus/bn.txt");
    assert_eq!(i.language.as_deref(), Some("bn"));
    assert!(i.title.is_some() && !i.bijoy && !i.has_cover);
    let i = info("fixtures/bijoy-sample.txt");
    assert!(i.bijoy);
}

#[test]
fn pdf_cover_fits_the_box() {
    let book = akkhor::open(path("corpus/books/pdf/bn.chrome.one.pdf")).unwrap();
    let c = book.cover(200, 300).unwrap();
    assert!(c.width <= 200 && c.height <= 300 && (c.width == 200 || c.height == 300));
    assert_eq!(c.rgba.len(), (c.width * c.height * 4) as usize);
}

/// The library reads info and a cover for every book it lists: under 50 ms
/// each, release build (debug builds are several times slower). The first
/// PDF of a session also pays one-time costs (reference fonts read for
/// Type 3 recognition, a few hundred ms), which a library import pays once.
#[test]
fn info_and_cover_are_quick() {
    if cfg!(debug_assertions) {
        return;
    }
    let books = [
        "corpus/books/pdf/bn.xelatex.two.pdf",
        "corpus/tables/pdf/bn.chrome.pdf",
        "fixtures/indic-sample.epub",
        "corpus/hi.txt",
    ];
    // Warm up the one-time costs on a different book.
    let _ = akkhor::open(path("corpus/tables/pdf/hi.chrome.pdf"))
        .unwrap()
        .info();
    for rel in books {
        let t = Instant::now();
        let book = akkhor::open(path(rel)).unwrap();
        let _ = book.info();
        let _ = book.cover(240, 360);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        assert!(ms < 50.0, "{rel}: {ms:.1} ms");
    }
}
