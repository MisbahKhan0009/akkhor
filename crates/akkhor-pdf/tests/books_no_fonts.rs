//! Typst writes its own text layer (ToUnicode maps glyphs to whole clusters,
//! ActualText spans cut mid-cluster): with no font known to Akkhor at all,
//! its words must still come out whole. A separate test binary, so no
//! reference font is registered.

mod common;

#[test]
fn typst_words_whole_without_fonts() {
    common::check_books(|n| n.contains("typst"), |_| 1.0);
}

/// With no font known, the text quality says which PDFs lose letters:
/// XeLaTeX (no ActualText; conjuncts only in the font's rules) falls below
/// the font-pack threshold, Chrome, Word and Typst (whose text layers name
/// every cluster) stay at 1, and a PDF with no text layer is at 0.
#[test]
fn text_quality_says_when_fonts_are_missing() {
    use akkhor_core::Source;
    use akkhor_pdf::PdfDocument;
    let q = |name: &str| {
        let doc = PdfDocument::open(&Source::open(common::corpus(name)).unwrap()).unwrap();
        doc.text_quality(0..3).unwrap()
    };
    for low in [
        "books/pdf/bn.xelatex.two.hind.pdf",
        "books/pdf/hi.xelatex.two.mukta.pdf",
    ] {
        assert!(q(low) < akkhor_pdf::TEXT_QUALITY_GOOD, "{low}: {}", q(low));
    }
    for good in [
        "books/pdf/bn.chrome.one.pdf",
        "books/pdf/bn.writer.one.kalpurush.pdf",
        "books/pdf/bn.typst.one.pdf",
    ] {
        assert_eq!(q(good), 1.0, "{good}");
    }
    assert_eq!(q("pdf/bn.chrome.raw.pdf"), 0.0);
}
