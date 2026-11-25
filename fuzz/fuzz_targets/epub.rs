//! An EPUB: open, metadata, cover, the first chapters.
#![no_main]
use akkhor_core::Source;
use akkhor_epub::EpubBook;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(book) = EpubBook::open(Source::from_vec(data.to_vec())) else {
        return;
    };
    let _ = book.cover_bytes();
    let _ = book.chapter_sizes();
    for i in 0..book.chapter_count().min(3) {
        if let Ok(doc) = book.chapter(i) {
            akkhor_fuzz::lay_out(&doc);
        }
    }
});
