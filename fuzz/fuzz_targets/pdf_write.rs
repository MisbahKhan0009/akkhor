//! Pages of a PDF copied as a new PDF, which must open and read again.
#![no_main]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(doc) = PdfDocument::open(&Source::from_vec(data.to_vec())) else {
        return;
    };
    let n = doc.page_count();
    if n == 0 {
        return;
    }
    let chosen = [n - 1, 0];
    let Ok(bytes) = doc.write_pages(&chosen) else {
        return;
    };
    let copy = PdfDocument::open(&Source::from_vec(bytes)).expect("a copy always opens");
    assert_eq!(copy.page_count(), 2);
    let _ = copy.page_text(0);
});
