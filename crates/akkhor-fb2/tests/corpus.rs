//! FB2 files: Calibre's, read against their EPUB source, and hand-written
//! ones covering what Calibre does not write (titles, nested sections,
//! poems, notes, pictures, Windows-1251, zipped).

use std::path::{Path, PathBuf};

use akkhor_core::Source;
use akkhor_epub::EpubBook;
use akkhor_fb2::Fb2Book;
use akkhor_flow::model::BlockKind;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/formats")
}

fn open(name: &str) -> Fb2Book {
    Fb2Book::open(Source::open(dir().join(name)).unwrap()).unwrap()
}

fn words(blocks: impl Iterator<Item = akkhor_flow::model::Block>) -> Vec<String> {
    blocks
        .filter(|b| !matches!(b.kind, BlockKind::Image))
        .flat_map(|b| {
            b.text
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn calibre_fb2_reads_like_its_source() {
    for lang in ["bn", "hi", "en"] {
        let src =
            EpubBook::open(Source::open(dir().join(format!("src/{lang}.epub"))).unwrap()).unwrap();
        let truth = words((0..src.chapter_count()).flat_map(|i| src.chapter(i).unwrap().blocks));
        let book = open(&format!("{lang}.fb2"));
        let got = words((0..book.chapter_count()).flat_map(|i| book.chapter(i).unwrap().blocks));
        // Calibre's FB2 keeps every word; the footnote mark is plain text.
        assert_eq!(got, truth, "{lang}.fb2");
        assert_eq!(book.metadata.title, src.metadata.title);
        assert_eq!(book.metadata.authors, src.metadata.creators);
        assert_eq!(book.metadata.language.as_deref(), Some(lang));
        assert_eq!(book.metadata.publisher.as_deref(), Some("Akkhor"));
        assert!(
            book.cover_bytes()
                .is_some_and(|b| b.starts_with(b"\x89PNG"))
        );
        // Calibre writes chapter headings as bold first paragraphs: they
        // become headings and the contents.
        let toc: Vec<String> = book.toc().iter().map(|t| t.title.clone()).collect();
        let want: Vec<String> = src.toc.iter().map(|t| t.title.clone()).collect();
        assert_eq!(toc, want, "{lang}.fb2 contents");
        for t in book.toc() {
            let first = book.chapter(t.chapter).unwrap().blocks.remove(0);
            assert!(matches!(first.kind, BlockKind::Heading(_)), "{}", t.title);
        }
    }
}

#[test]
fn hand_written_fb2_features() {
    let book = open("rich.bn.fb2");
    let m = &book.metadata;
    assert_eq!(m.title.as_deref(), Some("কবিতা ও গল্প"));
    assert_eq!(m.authors, ["অক্ষর পরীক্ষা", "দ্বিতীয় লেখক"]);
    assert_eq!(m.publisher.as_deref(), Some("অক্ষর প্রকাশনী"));
    assert_eq!(m.series.as_deref(), Some("অক্ষর সিরিজ"));
    assert_eq!(m.annotation.as_deref(), Some("একটি ছোট পরীক্ষার বই।"));
    let toc: Vec<(String, usize)> = book
        .toc()
        .into_iter()
        .map(|t| (t.title, t.chapter))
        .collect();
    assert_eq!(
        toc,
        [
            ("প্রথম অংশ সকালের গান".to_string(), 1),
            ("দ্বিতীয় অংশ".to_string(), 2),
            ("টীকা".to_string(), 3)
        ]
    );
    let ch1 = book.chapter(1).unwrap();
    let kinds: Vec<_> = ch1
        .blocks
        .iter()
        .map(|b| (b.kind, b.text.clone()))
        .collect();
    assert_eq!(
        kinds[0],
        (BlockKind::Heading(1), "প্রথম অংশ\nসকালের গান".to_string())
    );
    assert!(
        kinds.contains(&(BlockKind::Heading(4), "ছড়া".to_string())),
        "poem title"
    );
    assert!(
        kinds.contains(&(BlockKind::Heading(2), "উপ-অংশ".to_string())),
        "nested section"
    );
    assert!(
        kinds.iter().any(|(k, _)| matches!(k, BlockKind::Image)),
        "picture"
    );
    // Pictures decode from base64.
    let images = book.chapter_images(1);
    use akkhor_flow::image::ImageProvider;
    assert!(
        images
            .bytes("fb2:pic.png")
            .is_some_and(|b| b.starts_with(b"\x89PNG"))
    );
    assert!(book.cover_bytes().is_some());
    // The notes body is its own chapter.
    let notes = book.chapter(3).unwrap();
    assert!(notes.blocks.iter().any(|b| b.text == "গ্রামটি নদীর পূর্ব পাড়ে।"));

    // Zipped: the same book.
    let zipped = open("rich.bn.fb2.zip");
    assert_eq!(zipped.metadata.title, book.metadata.title);
    assert_eq!(zipped.chapter_count(), book.chapter_count());

    // Windows-1251.
    let ru = open("cp1251.ru.fb2");
    assert_eq!(ru.metadata.title.as_deref(), Some("Деревня у реки"));
    let text = &ru.chapter(0).unwrap().blocks[1].text;
    assert_eq!(text, "Утром над рекой стоит туман. «Ёлка» — №1.");
}

#[test]
fn damaged_and_hostile_fb2() {
    let good = std::fs::read(dir().join("rich.bn.fb2")).unwrap();
    for cut in (0..good.len()).step_by(97) {
        if let Ok(b) = Fb2Book::open(Source::from_vec(good[..cut].to_vec())) {
            let _ = b.cover_bytes();
            for i in 0..b.chapter_count() {
                let _ = b.chapter(i);
            }
        }
    }
    // Entities are not expanded beyond XML's own five, so a "billion laughs"
    // file stays flat.
    let laughs = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]>
        <FictionBook><description><title-info><book-title>&lol2;</book-title></title-info></description>
        <body><section><p>&lol2;&lol2;&lol2;</p></section></body></FictionBook>"#;
    let b = Fb2Book::open(Source::from_vec(laughs.as_bytes().to_vec())).unwrap();
    let doc = b.chapter(0).unwrap();
    assert!(doc.blocks.iter().map(|b| b.text.len()).sum::<usize>() < 100);
    // Not FictionBook at all.
    assert!(Fb2Book::open(Source::from_vec(b"<html><body>x</body></html>".to_vec())).is_err());
}
