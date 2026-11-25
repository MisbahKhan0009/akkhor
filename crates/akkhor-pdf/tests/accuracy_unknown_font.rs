//! PDFs whose font the engine does not know (Hind Siliguri and Mukta are
//! not registered here): there is nothing to recover from, but the result
//! must never be worse than the PDF's own text layer.

mod common;

use std::sync::Arc;

use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use common::*;

#[test]
fn unknown_fonts_are_no_worse_than_the_text_layer() {
    // Only the Noto fonts: the second designs stay unknown.
    for f in [
        "../../fonts/NotoSerifBengali-Regular.ttf",
        "../../fonts/NotoSerifDevanagari-Regular.ttf",
    ] {
        akkhor_pdf::register_reference_font(Arc::new(std::fs::read(corpus(f)).unwrap()));
    }
    for (stem, design) in [
        ("bn", "hind"),
        ("hi", "mukta"),
        ("bn-conjuncts", "hind"),
        ("hi-conjuncts", "mukta"),
    ] {
        let truth = std::fs::read_to_string(corpus(&format!("{stem}.txt"))).unwrap();
        for producer in ["chrome", "libreoffice"] {
            let file = format!("pdf/{stem}.{design}.{producer}.pdf");
            let pdf = PdfDocument::open(&Source::open(corpus(&file)).unwrap()).unwrap();
            let before = cer(&truth, &document_text(&pdf, true));
            let after = cer(&truth, &document_text(&pdf, false));
            println!(
                "{file:<48} {:>9.1}% {:>9.1}%",
                before * 100.0,
                after * 100.0
            );
            assert!(
                after <= before + 0.005,
                "{file}: {:.1}% is worse than the text layer's {:.1}%",
                after * 100.0,
                before * 100.0
            );
        }
    }
}
