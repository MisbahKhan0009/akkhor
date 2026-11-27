//! Reflowed screens drawn from glyph outlines and from cached masks: time
//! (first and repeat draws) and how far the pixels differ.
//!
//! `cargo run --release -p akkhor --example drawtime -- BOOK [SCREENS]`
//! Prints one CSV row per book: format, screens, background-only draw ms,
//! then first ms, next-screen ms, repeat ms and cache KB for outlines and for masks, then
//! the largest channel difference, channels differing by more than 8, and
//! the mean difference.

use std::time::Instant;

use akkhor::Theme;
use akkhor::flow::{FontSet, GlyphCache, LayoutOptions, paginate, render_page};

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args.next().expect("a book");
    let want: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let (w, h, size) = (1080u32, 2200u32, 42.0f32);
    let book = akkhor::open(&file).expect("open");
    let mut reading = akkhor::Reading::new(&book);
    let fonts = FontSet::bundled();
    let opts = LayoutOptions {
        width: w as f32,
        height: h as f32,
        font_size: size,
        margin_x: w as f32 * 0.067,
        ..Default::default()
    };
    let mut pages = Vec::new();
    for c in 0..reading.chapter_count() {
        let doc = reading.chapter(&book, c).expect("chapter").document.clone();
        pages.extend(paginate(&doc, &opts, &fonts));
        if pages.len() >= want {
            break;
        }
    }
    pages.truncate(want);
    let n = pages.len().max(1) as f64;
    let mut shots: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut row = Vec::new();
    for masks in [false, true] {
        let mut cache = GlyphCache::default();
        cache.use_masks = masks;
        let mut out = Vec::new();
        // Turning pages: each screen drawn once, in order, into a buffer
        // already in use; the first one starts with an empty cache.
        let mut first = 0.0;
        let mut turned = 0.0;
        let mut px = vec![0u8; (w * h * 4) as usize];
        for (i, p) in pages.iter().enumerate() {
            let t = Instant::now();
            render_page(p, &fonts, None, &mut cache, &Theme::PAPER, w, h, &mut px).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            if i == 0 {
                first = ms;
            } else {
                turned += ms;
            }
            out.push(px.clone());
        }
        let turned = turned / (pages.len().max(2) - 1) as f64;
        // Repeat draws into one buffer, as an app redraws a bitmap: the
        // best of 15 rounds, against a noisy machine.
        let mut px = vec![0u8; (w * h * 4) as usize];
        let mut warm = f64::MAX;
        for _ in 0..15 {
            let t = Instant::now();
            for p in &pages {
                render_page(p, &fonts, None, &mut cache, &Theme::PAPER, w, h, &mut px).unwrap();
            }
            warm = warm.min(t.elapsed().as_secs_f64() * 1e3 / n);
        }
        row.push(format!(
            "{first:.2},{turned:.2},{warm:.2},{}",
            cache.used_bytes() / 1024
        ));
        shots.push(out);
    }
    // The floor: a screen with no text (only the background).
    let mut px = vec![0u8; (w * h * 4) as usize];
    let empty = akkhor::flow::Page::default();
    let mut floor = f64::MAX;
    for _ in 0..15 {
        let t = Instant::now();
        render_page(
            &empty,
            &fonts,
            None,
            &mut GlyphCache::default(),
            &Theme::PAPER,
            w,
            h,
            &mut px,
        )
        .unwrap();
        floor = floor.min(t.elapsed().as_secs_f64() * 1e3);
    }
    let (mut max, mut over8, mut sum, mut count) = (0u8, 0usize, 0u64, 0u64);
    for (a, b) in shots[0].iter().zip(&shots[1]) {
        for (x, y) in a.iter().zip(b) {
            let d = x.abs_diff(*y);
            max = max.max(d);
            over8 += usize::from(d > 8);
            sum += u64::from(d);
            count += 1;
        }
    }
    let name = std::path::Path::new(&file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("?");
    println!(
        "{name},{},{floor:.2},{},{},{max},{over8},{:.4}",
        pages.len(),
        row[0],
        row[1],
        sum as f64 / count.max(1) as f64
    );
}
