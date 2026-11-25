//! Standard PDF fonts from the app instead of the engine: small.pdf uses
//! Helvetica without embedding it. Run without the `standard-fonts`
//! feature too (CI does), where only a registered font can draw it.

use std::sync::Arc;

use akkhor_core::{Source, Theme};
use akkhor_pdf::PdfDocument;

fn ink(pdf: &PdfDocument) -> usize {
    let page = pdf.render_page(0, 1.0, &Theme::ORIGINAL).unwrap();
    page.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count()
}

#[test]
fn a_registered_font_draws_standard_font_text() {
    let path = format!(
        "{}/../../tests/fixtures/small.pdf",
        env!("CARGO_MANIFEST_DIR")
    );
    let before = ink(&PdfDocument::open(&Source::open(&path).unwrap()).unwrap());
    let font = format!(
        "{}/../../fonts/NotoSerif-Regular.ttf",
        env!("CARGO_MANIFEST_DIR")
    );
    // A face of the family stands in for the others ("Helvetica-Bold" for
    // "Helvetica").
    akkhor_pdf::register_standard_font("Helvetica-Bold", Arc::new(std::fs::read(font).unwrap()));
    let after = ink(&PdfDocument::open(&Source::open(&path).unwrap()).unwrap());
    // The registered font is used, embedded set or not (without one,
    // hayro draws placeholder boxes).
    assert!(after > 100, "no text drawn ({after} dark pixels)");
    assert_ne!(after, before, "the registered font was not used");
}
