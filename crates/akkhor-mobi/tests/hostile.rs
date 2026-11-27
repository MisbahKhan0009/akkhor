//! DRM, PalmDOC, and damaged Kindle files: named or read, never a panic
//! or a hang.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use akkhor_core::{Error, Source};
use akkhor_mobi::{Kind, MobiBook};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/formats")
}

fn read_all(bytes: Vec<u8>) {
    if let Ok(book) = MobiBook::open(Source::from_vec(bytes)) {
        let _ = book.cover_bytes();
        let _ = book.toc();
        for i in 0..book.chapter_count() {
            let _ = book.chapter(i);
        }
    }
}

#[test]
fn kindle_drm_is_named() {
    for f in ["en.kf7.mobi", "en.azw3"] {
        let mut b = std::fs::read(dir().join(f)).unwrap();
        // Record 0 starts at the first record offset; encryption at +12.
        let r0 = u32::from_be_bytes([b[78], b[79], b[80], b[81]]) as usize;
        b[r0 + 13] = 2;
        assert!(
            matches!(MobiBook::open(Source::from_vec(b)), Err(Error::Protected)),
            "{f}"
        );
    }
}

#[test]
fn palmdoc_text_books_read() {
    // A PalmDOC book: header, record 0, one uncompressed text record.
    let text = "PalmDOC books are plain text.\n\nSecond line.";
    let mut r0 = Vec::new();
    r0.extend_from_slice(&1u16.to_be_bytes()); // no compression
    r0.extend_from_slice(&0u16.to_be_bytes());
    r0.extend_from_slice(&(text.len() as u32).to_be_bytes());
    r0.extend_from_slice(&1u16.to_be_bytes());
    r0.extend_from_slice(&4096u16.to_be_bytes());
    r0.extend_from_slice(&0u32.to_be_bytes());
    let mut b = vec![0u8; 78];
    b[..7].copy_from_slice(b"example");
    b[60..68].copy_from_slice(b"TEXtREAd");
    b[77] = 2;
    let start = 78 + 16 + 2;
    b.extend_from_slice(&(start as u32).to_be_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&((start + r0.len()) as u32).to_be_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&r0);
    b.extend_from_slice(text.as_bytes());
    let book = MobiBook::open(Source::from_vec(b)).unwrap();
    assert_eq!(book.kind, Kind::PalmDoc);
    let doc = book.chapter(0).unwrap();
    let all: Vec<&str> = doc.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(all, ["PalmDOC books are plain text.", "Second line."]);
}

#[test]
fn damaged_kindle_files_never_panic() {
    let t = Instant::now();
    let mut n = 0;
    for f in ["bn.kf7.mobi", "bn.azw3", "hi.both.mobi", "en.plain.mobi"] {
        let good = std::fs::read(dir().join(f)).unwrap();
        // Cut short at many points.
        for cut in (0..good.len()).step_by(good.len() / 60 + 1) {
            read_all(good[..cut].to_vec());
            n += 1;
        }
        // Bytes changed all over, with a fixed pseudo-random sequence.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..300 {
            let mut b = good.clone();
            for _ in 0..8 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let at = (seed as usize) % b.len();
                b[at] = (seed >> 32) as u8;
            }
            read_all(b);
            n += 1;
        }
        // Header fields at their worst: record counts, offsets, sizes.
        for at in (60..200).step_by(2) {
            let mut b = good.clone();
            b[at] = 0xff;
            b[at + 1] = 0xff;
            read_all(b);
            n += 1;
        }
    }
    assert!(n > 1000);
    assert!(t.elapsed() < Duration::from_secs(60), "{:?}", t.elapsed());
}
