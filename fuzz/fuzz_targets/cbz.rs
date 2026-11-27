//! A comic archive. An input starting with an even byte ("PK" does) is the
//! archive itself; otherwise its parts, split at FF FE, become pictures
//! and a ComicInfo.xml in a stored zip, so the decoders and the page
//! order see well-formed archives. Open, cover, the first pages drawn
//! small and as a tile.
#![no_main]
use akkhor_cbz::CbzBook;
use akkhor_core::{Source, Theme, Tile};
use libfuzzer_sys::fuzz_target;

const NAMES: [&str; 6] = [
    "10.jpg",
    "2.png",
    "ComicInfo.xml",
    "x/1.webp",
    "a.gif",
    "page 3.jpeg",
];

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let bytes = if first % 2 == 0 {
        data.to_vec()
    } else {
        let mut parts = Vec::new();
        let mut start = 0;
        for i in 0..rest.len().saturating_sub(1) {
            if rest[i] == 0xFF && rest[i + 1] == 0xFE && i >= start {
                parts.push(&rest[start..i]);
                start = i + 2;
            }
        }
        parts.push(&rest[start..]);
        let files: Vec<(&str, &[u8])> = NAMES.iter().copied().zip(parts).collect();
        akkhor_fuzz::zip(&files)
    };
    let Ok(book) = CbzBook::open(Source::from_vec(bytes)) else {
        return;
    };
    let _ = book.cover_bytes();
    let scale = f32::from(first) / 255.0 * 2.0;
    for i in 0..book.page_count().min(3) {
        let Ok((w, h)) = book.page_size(i) else {
            continue;
        };
        std::hint::black_box((w, h));
        let _ = book.render_page(i, scale.min(0.25), &Theme::PAPER);
        let _ = book.render_tile(i, scale, Tile::new(16, 16, 48, 48), &Theme::NIGHT);
    }
    let _ = book.trim();
});
