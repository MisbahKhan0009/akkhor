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

/// A page turned by `/Rotate` (set on the page tree) is read upright, as
/// MuPDF and Poppler read it; its boxes are on the page as displayed.
#[test]
fn a_turned_page_is_read_in_lines() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/write/inherited.pdf"
    );
    let pdf = akkhor_pdf::PdfDocument::open(&akkhor_core::Source::open(path).unwrap()).unwrap();
    assert_eq!(pdf.page_size(0).unwrap(), (800.0, 400.0));
    let text = pdf.page_text(0).unwrap();
    assert_eq!(text.text.trim(), "Page one");
    // MuPDF's box for "Page" on the displayed page: x 692.8 to 725.8,
    // y 72 to 128 (the word runs down the turned page).
    let r = text.rects(0..4);
    assert_eq!(r.len(), 1);
    let r = r[0];
    // Across the line the boxes differ by font metrics (Akkhor 694 to 719).
    assert!(r.x0 > 688.0 && r.x1 < 730.0 && r.width() > 20.0, "{r:?}");
    assert!(
        (r.y0 - 72.0).abs() < 4.0 && (r.y1 - 128.0).abs() < 4.0,
        "{r:?}"
    );
    // And as reflowed text.
    let flow = pdf.reflow(0..1).unwrap();
    assert!(
        flow.document
            .blocks
            .iter()
            .any(|b| b.text.contains("Page one"))
    );
}

/// A picture cut from a turned page is given upright, as the page's text is
/// read: the words above the blue bar, the bar wide rather than tall.
#[test]
fn a_turned_pages_picture_is_upright() {
    use akkhor_flow::ImageProvider;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/write/inherited.pdf"
    );
    let pdf = PdfDocument::open(&Source::open(path).unwrap()).unwrap();
    let page = pdf
        .render_page(0, 1.0, &akkhor_core::Theme::ORIGINAL)
        .unwrap();
    // Everything drawn, on the page as displayed.
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..page.height {
        for x in 0..page.width {
            let i = ((y * page.width + x) * 4) as usize;
            if page.pixels[i] < 200 || page.pixels[i + 1] < 200 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    // Displayed, the content runs down the page: taller than wide.
    assert!(y1 - y0 > x1 - x0, "{x0},{y0} {x1},{y1}");
    // As reflow::image_source writes it.
    let src = format!("pdf:0:{x0}.0,{y0}.0,{x1}.0,{y1}.0");
    let images = pdf.reflow_images();
    let (w, h) = images.size(&src).unwrap();
    assert!(w > h, "given upright: {w} x {h}");
    let img = images.decoded(&src, w, h).unwrap();
    assert_eq!((img.width, img.height), (w, h));
    let row_kind = |y: u32| {
        let row = &img.rgba[(y * img.width * 4) as usize..((y + 1) * img.width * 4) as usize];
        let blue = row
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] > 200 && p[0] < 60);
        let ink = row
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] < 100 && p[2] < 100);
        (ink, blue)
    };
    let top = (0..img.height / 3).map(row_kind).collect::<Vec<_>>();
    let bottom = (img.height * 2 / 3..img.height)
        .map(row_kind)
        .collect::<Vec<_>>();
    assert!(
        top.iter().any(|r| r.0) && !top.iter().any(|r| r.1),
        "words on top"
    );
    assert!(
        bottom.iter().any(|r| r.1) && !bottom.iter().any(|r| r.0),
        "bar below"
    );
}
