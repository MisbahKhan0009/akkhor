//! A chapter's markup and style sheet, through an EPUB, then laid out.
//! The input is split at the first NUL: markup, then CSS.
#![no_main]
use akkhor_core::Source;
use akkhor_epub::EpubBook;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (markup, css) = match data.iter().position(|&b| b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let bytes = akkhor_fuzz::epub(markup, css);
    let book = EpubBook::open(Source::from_vec(bytes)).expect("the wrapper is a valid EPUB");
    if let Ok(doc) = book.chapter(0) {
        akkhor_fuzz::lay_out(&doc);
    }
});
