//! Comics: page order, every picture format, ComicInfo, drawing, junk and
//! damaged archives.

use std::path::{Path, PathBuf};

use akkhor_cbz::CbzBook;
use akkhor_core::{Source, Theme, Tile};

fn open(name: &str) -> CbzBook {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/formats")
        .join(name);
    CbzBook::open(Source::open(p).unwrap()).unwrap()
}

#[test]
fn pages_in_natural_order_in_every_format() {
    let book = open("mixed.cbz");
    // p1, p2, p3, p10, p11: junk entries left out, numbers by value.
    assert_eq!(book.page_count(), 5);
    let sizes: Vec<(f32, f32)> = (0..5).map(|i| book.page_size(i).unwrap()).collect();
    assert_eq!(sizes[3], (800.0, 600.0), "p10 is the wide spread");
    // JPEG, PNG, WebP, JPEG, GIF all draw, with the yellow disc in the middle.
    for (i, &(w, h)) in sizes.iter().enumerate() {
        let page = book.render_page(i, 0.5, &Theme::PAPER).unwrap();
        assert_eq!(
            (page.width, page.height),
            ((w * 0.5) as u32, (h * 0.5) as u32),
            "page {i}"
        );
        let c = (((page.height / 2) * page.width + page.width / 2) * 4) as usize;
        let px = &page.pixels[c..c + 4];
        assert!(
            px[0] > 200 && px[1] > 180 && px[2] < 120,
            "page {i} centre {px:?}"
        );
    }
    assert!(!book.metadata.right_to_left);
    assert!(
        book.cover_bytes()
            .is_some_and(|b| b.starts_with(&[0xff, 0xd8]))
    );
}

#[test]
fn comic_info_and_dark_themes() {
    let book = open("manga.cbz");
    let m = &book.metadata;
    assert_eq!(m.title.as_deref(), Some("নদীর গল্প"));
    assert_eq!(m.series.as_deref(), Some("অক্ষর কমিক"));
    assert_eq!(m.number.as_deref(), Some("3"));
    assert_eq!(m.writers, ["লেখক এক", "Writer Two"]);
    assert_eq!(m.language.as_deref(), Some("bn"));
    assert!(m.right_to_left);
    assert_eq!(m.cover_page, 2);
    assert_eq!(book.cover_bytes().unwrap(), book.page_bytes(2).unwrap());
    // Tiles at 2x match the same area of the whole page; dark themes dim.
    let whole = book.render_page(0, 2.0, &Theme::PAPER).unwrap();
    let tile = book
        .render_tile(0, 2.0, Tile::new(300, 500, 64, 64), &Theme::PAPER)
        .unwrap();
    for y in 0..64u32 {
        let a = ((y * 64) * 4) as usize;
        let b = (((500 + y) * whole.width + 300) * 4) as usize;
        assert_eq!(&tile.pixels[a..a + 256], &whole.pixels[b..b + 256]);
    }
    let night = book
        .render_tile(0, 2.0, Tile::new(300, 500, 1, 1), &Theme::NIGHT)
        .unwrap();
    let day = &tile.pixels[..4];
    for (c, &v) in day.iter().take(3).enumerate() {
        let want = (u32::from(v) * 217) >> 8;
        assert!((i32::from(night.pixels[c]) - want as i32).abs() <= 1);
    }
    assert!(book.trim() > 0);
}

#[test]
fn rar_7z_and_damaged_archives() {
    assert!(CbzBook::open(Source::from_vec(b"Rar!\x1a\x07\x00rest".to_vec())).is_err());
    assert!(CbzBook::open(Source::from_vec(b"7z\xbc\xaf\x27\x1crest".to_vec())).is_err());
    let good = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/formats/mixed.cbz"),
    )
    .unwrap();
    for cut in (0..good.len()).step_by(4999) {
        if let Ok(b) = CbzBook::open(Source::from_vec(good[..cut].to_vec())) {
            for i in 0..b.page_count() {
                let _ = b.render_page(i, 0.1, &Theme::PAPER);
            }
        }
    }
    let mut seed = 7u64;
    for _ in 0..60 {
        let mut b = good.clone();
        for _ in 0..6 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let at = (seed >> 33) as usize % b.len();
            b[at] = (seed >> 13) as u8;
        }
        if let Ok(book) = CbzBook::open(Source::from_vec(b)) {
            for i in 0..book.page_count() {
                let _ = book.render_page(i, 0.1, &Theme::PAPER);
            }
        }
    }
}

/// Two pages drawn in turn, tile by tile (the page on screen and the one
/// drawn ahead), from several threads: each page is decoded once and kept;
/// trim frees both.
#[test]
fn pages_drawn_side_by_side_stay_decoded() {
    let book = std::sync::Arc::new(open("mixed.cbz"));
    std::thread::scope(|s| {
        for t in 0..4u32 {
            let book = book.clone();
            s.spawn(move || {
                for k in 0..8u32 {
                    let page = ((t + k) % 2) as usize;
                    let tile = Tile::new(8 * k, 8 * t, 16, 16);
                    book.render_tile(page, 0.5, tile, &Theme::PAPER).unwrap();
                }
            });
        }
    });
    // Drawn again: no decode, so fast even in a debug build.
    let t = std::time::Instant::now();
    for _ in 0..20 {
        for page in 0..2 {
            book.render_tile(page, 0.5, Tile::new(0, 0, 4, 4), &Theme::PAPER)
                .unwrap();
        }
    }
    let again = t.elapsed();
    assert!(book.trim() > 0);
    assert_eq!(book.trim(), 0);
    let t = std::time::Instant::now();
    book.render_tile(0, 0.5, Tile::new(0, 0, 4, 4), &Theme::PAPER)
        .unwrap();
    let decode = t.elapsed();
    // 40 cached draws cost less than one decode.
    assert!(again < decode, "{again:?} vs one decode {decode:?}");
}
