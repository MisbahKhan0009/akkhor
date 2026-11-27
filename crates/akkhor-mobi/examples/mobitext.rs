//! Prints a MOBI book's metadata, contents and chapter text.
//!   cargo run -p akkhor-mobi --example mobitext -- book.azw3

use akkhor_core::Source;
use akkhor_mobi::MobiBook;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let book = MobiBook::open(Source::open(&path).unwrap()).unwrap();
    println!("kind: {:?}", book.kind);
    println!("{:?}", book.metadata);
    println!("cover: {:?} bytes", book.cover_bytes().map(|b| b.len()));
    for t in book.toc() {
        println!("toc: {} {} -> {}", t.depth, t.title, t.chapter);
    }
    for i in 0..book.chapter_count() {
        let doc = book.chapter(i).unwrap();
        let text: Vec<String> = doc
            .blocks
            .iter()
            .map(|b| format!("[{:?}] {}", b.kind, b.text))
            .collect();
        println!(
            "--- chapter {i}: {} blocks\n{}",
            doc.blocks.len(),
            text.join("\n")
        );
    }
}
