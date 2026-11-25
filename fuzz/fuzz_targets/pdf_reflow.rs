//! A PDF as reflowed text: chapters, then the first chapter's first pages.
#![no_main]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(doc) = PdfDocument::open(&Source::from_vec(data.to_vec())) else {
        return;
    };
    let chapters = doc.reflow_chapters();
    if let Some(first) = chapters.first() {
        let end = first.end.min(first.start + 3);
        if let Ok(r) = doc.reflow(first.start..end) {
            std::hint::black_box(r);
        }
    }
});
