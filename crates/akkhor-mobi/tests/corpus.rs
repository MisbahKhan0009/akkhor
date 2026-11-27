//! Kindle books written by Calibre from our EPUB sources
//! (tests/corpus/formats, scripts/make_format_corpus.py) read as their
//! source does: every word in order, the chapters' headings, the table of
//! contents, metadata and cover.

use std::path::{Path, PathBuf};

use akkhor_core::Source;
use akkhor_epub::EpubBook;
use akkhor_mobi::{Kind, MobiBook};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/formats")
}

fn words_of_epub(lang: &str) -> Vec<String> {
    let book =
        EpubBook::open(Source::open(dir().join(format!("src/{lang}.epub"))).unwrap()).unwrap();
    (0..book.chapter_count())
        .flat_map(|i| book.chapter(i).unwrap().blocks)
        // Picture alt text: MOBI 7 has none.
        .filter(|b| !matches!(b.kind, akkhor_flow::model::BlockKind::Image))
        .flat_map(|b| {
            b.text
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn words_of(book: &MobiBook) -> Vec<String> {
    (0..book.chapter_count())
        .flat_map(|i| book.chapter(i).unwrap().blocks)
        // Picture alt text: MOBI 7 has none.
        .filter(|b| !matches!(b.kind, akkhor_flow::model::BlockKind::Image))
        .flat_map(|b| {
            b.text
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Share of `truth`'s words found in `got` in the same order.
fn in_order(truth: &[String], got: &[String]) -> f64 {
    let (n, m) = (truth.len(), got.len());
    let mut prev = vec![0u32; m + 1];
    for word in truth {
        let mut cur = vec![0u32; m + 1];
        for j in 0..m {
            cur[j + 1] = if *word == got[j] {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        prev = cur;
    }
    f64::from(prev[m]) / n.max(1) as f64
}

#[test]
fn kindle_books_read_like_their_source() {
    for lang in ["bn", "hi", "en"] {
        let truth = words_of_epub(lang);
        let source =
            EpubBook::open(Source::open(dir().join(format!("src/{lang}.epub"))).unwrap()).unwrap();
        for (file, kind) in [
            ("kf7.mobi", Kind::Kf7),
            ("plain.mobi", Kind::Kf7),
            ("azw3", Kind::Kf8),
            ("both.mobi", Kind::Kf8),
        ] {
            let name = format!("{lang}.{file}");
            let book = MobiBook::open(Source::open(dir().join(&name)).unwrap()).unwrap();
            assert_eq!(book.kind, kind, "{name}");
            let got = words_of(&book);
            let score = in_order(&truth, &got);
            if score < 1.0 {
                let missing: Vec<&String> = truth.iter().filter(|w| !got.contains(w)).collect();
                eprintln!("{name}: not found: {missing:?}");
            }
            assert_eq!(
                score, 1.0,
                "{name}: {:.3} of the source's words in order",
                score
            );
            assert_eq!(book.metadata.title, source.metadata.title, "{name}");
            assert_eq!(book.metadata.authors, source.metadata.creators, "{name}");
            assert_eq!(book.metadata.language.as_deref(), Some(lang), "{name}");
            assert!(book.cover_bytes().is_some(), "{name}: cover");
            let toc: Vec<String> = book.toc().iter().map(|t| t.title.clone()).collect();
            let want: Vec<String> = source.toc.iter().map(|t| t.title.clone()).collect();
            assert_eq!(toc, want, "{name}: contents");
            // Contents point at chapters that start with their heading.
            for t in book.toc() {
                let first = book
                    .chapter(t.chapter)
                    .unwrap()
                    .blocks
                    .into_iter()
                    .next()
                    .unwrap();
                assert_eq!(
                    first.text.trim(),
                    t.title,
                    "{name}: {} opens its chapter",
                    t.title
                );
                assert!(
                    matches!(first.kind, akkhor_flow::model::BlockKind::Heading(_)),
                    "{name}: heading"
                );
            }
        }
    }
}
