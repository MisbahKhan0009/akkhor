//! A PDF: open, describe, read the text of the first pages, draw small.
#![no_main]
use akkhor_core::{Source, Theme};
use akkhor_pdf::PdfDocument;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(doc) = PdfDocument::open(&Source::from_vec(data.to_vec())) else {
        return;
    };
    let _ = doc.info();
    let _ = doc.outline();
    let n = doc.page_count().min(2);
    for i in 0..n {
        let _ = doc.page_text(i);
        let _ = doc.thumbnail(i, 48, &Theme::ORIGINAL);
    }
    let _ = doc.text_quality(0..n);
});
