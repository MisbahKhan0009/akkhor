//! A FictionBook: plain (first byte even) or zipped. Open (encoding,
//! metadata), contents, cover, the first chapters laid out, their pictures
//! (base64 binaries).
#![no_main]
use akkhor_core::Source;
use akkhor_fb2::Fb2Book;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let bytes = if first % 2 == 0 {
        data.to_vec()
    } else {
        akkhor_fuzz::zip(&[("book.fb2", rest)])
    };
    let Ok(book) = Fb2Book::open(Source::from_vec(bytes)) else {
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
