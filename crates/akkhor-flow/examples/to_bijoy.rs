//! Unicode Bangla on stdin to Bijoy on stdout, with the `poriborton` crate
//! (used by scripts/make_corpus.py to build the Bijoy PDFs).
use std::io::Read;

fn main() {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).unwrap();
    let composed = text
        .replace("\u{09A1}\u{09BC}", "\u{09DC}")
        .replace("\u{09A2}\u{09BC}", "\u{09DD}")
        .replace("\u{09AF}\u{09BC}", "\u{09DF}");
    print!("{}", poriborton::bijoy2000::unicode_to_bijoy(&composed));
}
