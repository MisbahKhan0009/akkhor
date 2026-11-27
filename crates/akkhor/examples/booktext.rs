//! Every chapter's text of a reflowable book, in order, for scoring
//! against other engines. Prints the text to stdout and
//! `open_ms,text_ms,chapters` to stderr.
//!
//! `cargo run --release -p akkhor --example booktext -- BOOK`

use std::time::Instant;

fn main() {
    let file = std::env::args().nth(1).expect("a book");
    let t = Instant::now();
    let book = akkhor::open(&file).expect("open");
    let open_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let flow = book.flow().expect("a reflowable book");
    let n = flow.chapter_count();
    let mut out = String::new();
    for i in 0..n {
        let Ok(doc) = flow.chapter(i) else { continue };
        for b in &doc.blocks {
            if b.image.is_none() {
                out.push_str(&b.text);
                out.push('\n');
            }
        }
    }
    let text_ms = t.elapsed().as_secs_f64() * 1e3;
    print!("{out}");
    eprintln!("{open_ms:.2},{text_ms:.2},{n}");
}
