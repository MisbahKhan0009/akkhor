//! Producer quirks seen in real-world Bangla PDFs, reproduced on corpus
//! files by scripts/make_producer_quirks.py: fake bold (text drawn twice on
//! itself) and font programs rewritten so outlines shift sideways. Both
//! start from PDFs without any text layer, so the text comes only from
//! recovering the glyphs.

mod common;

use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use common::*;

#[test]
fn quirks_read_exactly() {
    register_test_fonts();
    for (stem, truth) in [
        ("bn.chrome", "bn.txt"),
        ("hi.libreoffice", "hi.txt"),
        ("bn.hind.libreoffice", "bn.txt"),
    ] {
        let truth = std::fs::read_to_string(corpus(truth)).unwrap();
        for quirk in ["fakebold", "rewritten"] {
            let file = format!("pdf/{stem}.{quirk}.raw.pdf");
            let pdf = PdfDocument::open(&Source::open(corpus(&file)).unwrap()).unwrap();
            let got = document_text(&pdf, false);
            let e = cer(&truth, &got);
            println!("{file:44} {:5.2}%", e * 100.0);
            assert!(e < 0.005, "{file}: {:.1}% of characters wrong", e * 100.0);
        }
    }
}

#[test]
fn fake_bold_is_reported_as_bold() {
    let pdf = PdfDocument::open(&Source::open(corpus("pdf/bn.chrome.fakebold.raw.pdf")).unwrap())
        .unwrap();
    let glyphs = pdf.glyphs(0).unwrap();
    assert!(!glyphs.glyphs.is_empty());
    assert!(glyphs.glyphs.iter().all(|g| g.bold));
}

#[test]
fn synthetic_bold_type3_headings_are_recognised() {
    register_test_fonts();
    let pdf = PdfDocument::open(&Source::open(corpus("pdf/indic.raw.pdf")).unwrap()).unwrap();
    let text = pdf.page_text(0).unwrap().text;
    assert!(text.contains("অক্ষর পরীক্ষা"), "{text}");
    assert!(text.contains("अक्षर परीक्षण"), "{text}");
}

/// A flipped byte inside a page's /MediaBox numbers used to make the whole
/// page dictionary unreadable, so the page (here, the document) was lost.
/// MuPDF skips the bad token; so does Akkhor now.
#[test]
fn a_damaged_number_does_not_lose_the_page() {
    // A LibreOffice PDF: the page dictionary holds its own /MediaBox.
    let mut data = std::fs::read(corpus("pdf/hi.libreoffice.pdf")).unwrap();
    let at = data
        .windows(16)
        .position(|w| w == b"/MediaBox[0 0 59")
        .unwrap();
    data[at + 17] = 0xC5; // 595.3 -> 595\xC5.3
    let doc = PdfDocument::open(&Source::from_vec(data)).unwrap();
    assert!(doc.page_count() >= 1);
    let page = doc
        .render_page(0, 0.5, &akkhor_core::Theme::ORIGINAL)
        .unwrap();
    assert!(page.width > 0 && page.height > 0);
}
