//! Inputs the fuzzer found for the Kindle, FictionBook and comic targets
//! (tests/corpus/fuzz/*.{mobi,fb2,cbz}.bin): each must go through the
//! target's steps quickly, without panicking. PDFs have their own test in
//! akkhor-pdf.

use std::path::Path;
use std::time::{Duration, Instant};

use akkhor::flow::ImageProvider;
use akkhor::{Source, Theme, Tile};

fn pictures(doc: &akkhor::flow::Document, images: &dyn ImageProvider) {
    for src in doc.blocks.iter().filter_map(|b| b.image.as_deref()).take(4) {
        let _ = images.size(src);
        let _ = images.decoded(src, 64, 64);
    }
}

fn mobi(data: Vec<u8>) {
    let Ok(book) = akkhor::mobi::MobiBook::open(Source::from_vec(data)) else {
        return;
    };
    let _ = (book.toc(), book.cover_bytes(), book.chapter_sizes());
    for i in 0..book.chapter_count().min(3) {
        if let Ok(doc) = book.chapter(i) {
            pictures(&doc, &book.chapter_images(i));
        }
    }
}

fn fb2(data: Vec<u8>) {
    let Ok(book) = akkhor::fb2::Fb2Book::open(Source::from_vec(data)) else {
        return;
    };
    let _ = (book.toc(), book.cover_bytes(), book.chapter_sizes());
    for i in 0..book.chapter_count().min(3) {
        if let Ok(doc) = book.chapter(i) {
            pictures(&doc, &book.chapter_images(i));
        }
    }
}

fn cbz(data: Vec<u8>) {
    let Ok(book) = akkhor::cbz::CbzBook::open(Source::from_vec(data)) else {
        return;
    };
    let _ = book.cover_bytes();
    for i in 0..book.page_count().min(3) {
        let _ = book.page_size(i);
        let _ = book.render_page(i, 0.25, &Theme::PAPER);
        let _ = book.render_tile(i, 1.0, Tile::new(16, 16, 48, 48), &Theme::NIGHT);
    }
}

#[test]
fn inputs_found_by_fuzzing_finish_quickly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/fuzz");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let name = p.to_string_lossy().to_string();
        let run: fn(Vec<u8>) = if name.ends_with(".mobi.bin") {
            mobi
        } else if name.ends_with(".fb2.bin") {
            fb2
        } else if name.ends_with(".cbz.bin") {
            cbz
        } else {
            continue;
        };
        let t = Instant::now();
        run(std::fs::read(&p).unwrap());
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "{name}: {:?}",
            t.elapsed()
        );
        n += 1;
    }
    assert!(n >= 1);
}
