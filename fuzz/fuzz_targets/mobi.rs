//! A Kindle book (MOBI 7, AZW3, combined, PalmDOC): open (PalmDB, header,
//! EXTH), contents (INDX), cover, the first chapters (both decompressors,
//! KF8 skeleton and fragments) laid out, and their pictures.
#![no_main]
use akkhor_core::Source;
use akkhor_mobi::MobiBook;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(book) = MobiBook::open(Source::from_vec(data.to_vec())) else {
        return;
    };
    std::hint::black_box(&book.metadata);
    let _ = book.toc();
    let _ = book.cover_bytes();
    let _ = book.chapter_sizes();
    for i in 0..book.chapter_count().min(3) {
        if let Ok(doc) = book.chapter(i) {
            akkhor_fuzz::lay_out(&doc);
            akkhor_fuzz::pictures(&doc, &book.chapter_images(i));
        }
    }
});
