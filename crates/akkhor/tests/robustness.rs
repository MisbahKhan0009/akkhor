//! Books are untrusted input. This test feeds damaged and random files to
//! every part of the engine and checks that nothing panics. It is a cheap,
//! deterministic stand-in for fuzzing (cargo-fuzz targets come in Phase 7).

use std::panic::{AssertUnwindSafe, catch_unwind};

use akkhor::flow::{
    FontSet, GlyphCache, LayoutOptions, find_all, paginate_with_images, render_page,
};
use akkhor::{Book, Source, Theme};

/// xorshift64*: deterministic, no dependencies.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Opens `bytes` and exercises everything the format offers.
fn exercise(bytes: Vec<u8>, name: &str, fonts: &FontSet) {
    let Ok(book) = akkhor::open_source(Source::from_vec(bytes), Some(name)) else {
        return;
    };
    let opts = LayoutOptions {
        width: 240.0,
        height: 360.0,
        margin_x: 12.0,
        margin_top: 12.0,
        margin_bottom: 12.0,
        font_size: 14.0,
        ..Default::default()
    };
    let mut cache = GlyphCache::new(1 << 20);
    let mut px = vec![0u8; 240 * 360 * 4];
    match &book {
        Book::Pdf(pdf) => {
            for i in 0..pdf.page_count().min(2) {
                let _ = pdf.page_size(i);
                let _ = pdf.render_page(i, 0.3, &Theme::PAPER);
                // The text layer: recovery from damaged fonts, reordering,
                // search, selection.
                if let Ok(text) = pdf.page_text(i) {
                    let _ = text.find("ক্ষ");
                    let _ = text.offset_at(100.0, 100.0);
                    let _ = text.word_at(text.text.len() / 2);
                    let _ = text.snap(text.text.len() / 3);
                    let _ = text.rects(0..text.text.len());
                }
            }
            let _ = pdf.search("কর", 0).take(3).count();
            let _ = pdf.outline();
            let _ = pdf.thumbnail(0, 64, &Theme::NIGHT);
            // Reflow of the first chapter, laid out, and its positions.
            if let Some(range) = pdf.reflow_chapters().first().cloned()
                && let Ok(r) = pdf.reflow(range)
            {
                let images = pdf.reflow_images();
                let opts = akkhor::flow::LayoutOptions {
                    width: 320.0,
                    height: 480.0,
                    ..Default::default()
                };
                let pages =
                    akkhor::flow::paginate_with_images(&r.document, &opts, fonts, Some(&images));
                assert!(!pages.is_empty());
                for (b, block) in r.document.blocks.iter().enumerate().take(20) {
                    for off in [0, block.text.len() / 2, block.text.len()] {
                        if let Some((page, at)) = r.to_page(b, off) {
                            let _ = r.from_page(page, at);
                        }
                    }
                }
            }
        }
        Book::Epub(epub) => {
            let _ = epub.toc.len();
            for c in 0..epub.chapter_count().min(4) {
                let Ok(doc) = epub.chapter(c) else { continue };
                let images = epub.chapter_images(c);
                let pages = paginate_with_images(&doc, &opts, fonts, Some(&images));
                if let Some(p) = pages.first() {
                    let _ = render_page(
                        p,
                        fonts,
                        Some(&images),
                        &mut cache,
                        &Theme::NIGHT,
                        240,
                        360,
                        &mut px,
                    );
                }
                let _ = find_all(&doc, "ক্ষ");
            }
        }
        Book::Text(t) => {
            let pages = paginate_with_images(&t.document, &opts, fonts, None);
            if let Some(p) = pages.first() {
                let _ = render_page(p, fonts, None, &mut cache, &Theme::PAPER, 240, 360, &mut px);
            }
            let _ = find_all(&t.document, "আমি");
        }
    }
}

fn run(name: &str, bytes: &[u8], rng: &mut Rng, mutations: usize, fonts: &FontSet) {
    let check = |data: Vec<u8>, what: &str| {
        let r = catch_unwind(AssertUnwindSafe(|| exercise(data, name, fonts)));
        assert!(r.is_ok(), "panic on {name} ({what})");
    };
    check(bytes.to_vec(), "original");
    for i in 0..mutations {
        let mut data = bytes.to_vec();
        match i % 4 {
            // Truncate.
            0 => data.truncate(rng.below(data.len())),
            // Flip a few bytes.
            1 => {
                for _ in 0..1 + rng.below(8) {
                    let at = rng.below(data.len());
                    data[at] = rng.next() as u8;
                }
            }
            // Overwrite a run with 0xFF (large lengths and offsets).
            2 => {
                let at = rng.below(data.len());
                let end = (at + 1 + rng.below(16)).min(data.len());
                data[at..end].fill(0xFF);
            }
            // Duplicate a slice somewhere else.
            _ => {
                let a = rng.below(data.len());
                let b = (a + rng.below(256)).min(data.len());
                let slice = data[a..b].to_vec();
                let at = rng.below(data.len());
                data.splice(at..at, slice);
            }
        }
        check(data, &format!("mutation {i}"));
    }
}

#[test]
fn damaged_files_never_panic() {
    let fonts = FontSet::bundled();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    run(
        "sample.epub",
        &fixture("indic-sample.epub"),
        &mut rng,
        400,
        &fonts,
    );
    run("small.pdf", &fixture("small.pdf"), &mut rng, 200, &fonts);
    run("indic.pdf", &fixture("indic.pdf"), &mut rng, 120, &fonts);
    // LibreOffice output: renumbered subsets, symbol character maps.
    let lo = std::fs::read(format!(
        "{}/../../tests/corpus/pdf/hi.libreoffice.pdf",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    run("hi.libreoffice.pdf", &lo, &mut rng, 40, &fonts);
    // Book layouts (columns, Typst's tagged artifacts and ActualText spans,
    // Type 3 bold) through page structure and reflow.
    for book in [
        "bn.typst.two.pdf",
        "bn.chrome.two.pdf",
        "hi.xelatex.two.pdf",
    ] {
        let data = std::fs::read(format!(
            "{}/../../tests/corpus/books/pdf/{book}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        run(book, &data, &mut rng, 30, &fonts);
    }
    run(
        "bijoy.txt",
        &fixture("bijoy-sample.txt"),
        &mut rng,
        100,
        &fonts,
    );
}

#[test]
fn random_bytes_never_panic() {
    let fonts = FontSet::bundled();
    let mut rng = Rng(42);
    for len in [0usize, 1, 4, 22, 64, 512, 4096] {
        for prefix in [
            &b""[..],
            b"%PDF-1.7\n",
            b"PK\x03\x04",
            b"\xEF\xBB\xBF",
            b"\xFF\xFE",
        ] {
            let mut data = prefix.to_vec();
            data.extend((0..len).map(|_| rng.next() as u8));
            let r = catch_unwind(AssertUnwindSafe(|| exercise(data, "x.bin", &fonts)));
            assert!(
                r.is_ok(),
                "panic on random input, len {len}, prefix {prefix:?}"
            );
        }
    }
}

/// Hostile but well-formed inputs: huge nesting, huge paragraphs, odd CSS.
#[test]
fn hostile_documents_stay_fast() {
    use akkhor::flow::{Document, paginate};
    use std::time::Instant;
    let fonts = FontSet::bundled();
    let opts = LayoutOptions::default();
    // Time limits are for release builds; debug builds are ~10x slower.
    let slack = if cfg!(debug_assertions) { 10.0 } else { 1.0 };

    let deep = format!("{}text{}", "<div>".repeat(20_000), "</div>".repeat(20_000));
    let t = Instant::now();
    let doc = Document::from_html(&deep);
    assert_eq!(doc.blocks.len(), 1);
    assert!(
        t.elapsed().as_secs_f32() < 0.5 * slack,
        "deep nesting took {:?}",
        t.elapsed()
    );

    // A text file with one paragraph per line and no blank lines: each line
    // must become its own paragraph, so the first page is instant.
    let line = "আমি বাংলায় গান গাই আমি বাংলার গান গাই\n";
    let text = line.repeat(25_000);
    let t = Instant::now();
    let doc = Document::from_plain_text(&text);
    assert_eq!(doc.blocks.len(), 25_000);
    let mut p = akkhor::flow::Paginator::new(&doc, &opts, &fonts, None);
    assert!(p.next().is_some());
    assert!(
        t.elapsed().as_secs_f32() < 0.5 * slack,
        "first page took {:?}",
        t.elapsed()
    );

    // A single 200,000-word paragraph still lays out in linear time.
    let doc = Document::from_plain_text(&line.trim().repeat(25_000));
    let t = Instant::now();
    let pages = paginate(&doc, &opts, &fonts);
    assert!(pages.len() > 100);
    assert!(
        t.elapsed().as_secs_f32() < 3.0 * slack,
        "giant paragraph took {:?}",
        t.elapsed()
    );
}
