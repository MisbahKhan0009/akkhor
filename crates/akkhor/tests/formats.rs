//! MOBI, AZW3, FB2 and comics through the same calls as EPUB: library
//! card, cover, chapters, places, progress and whole-book search.

use akkhor::search::BookSearch;
use akkhor::{Book, Error, Format, Locator, Reading};

fn path(rel: &str) -> String {
    format!(
        "{}/../../tests/corpus/formats/{rel}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn reflowable_formats_share_one_interface() {
    for (file, format) in [
        ("bn.kf7.mobi", Format::Mobi),
        ("bn.azw3", Format::Mobi),
        ("bn.both.mobi", Format::Mobi),
        ("bn.fb2", Format::Fb2),
        ("rich.bn.fb2.zip", Format::Fb2),
    ] {
        let book = akkhor::open(path(file)).unwrap();
        assert_eq!(book.format(), format, "{file}");
        assert!(!book.is_paged());
        let info = book.info();
        assert_eq!(info.language.as_deref(), Some("bn"), "{file}");
        assert!(info.title.is_some() && !info.authors.is_empty(), "{file}");
        assert!(info.has_cover && info.reflowable, "{file}");
        let cover = book.cover(120, 180).unwrap();
        assert!(cover.width <= 120 && cover.height <= 180, "{file}");
        let json = info.to_json();
        assert!(json.contains(&format!(
            "\"format\":\"{}\"",
            if format == Format::Mobi {
                "mobi"
            } else {
                "fb2"
            }
        )));
        // Places and progress: start, middle, end.
        let mut r = Reading::new(&book);
        assert!(r.chapter_count() >= 3, "{file}");
        let mid = r.at_progress(&book, 0.5).unwrap();
        let p = r.progress(&book, mid).unwrap();
        assert!((p - 0.5).abs() < 0.02, "{file}: {p}");
        let end = r
            .clamp(
                &book,
                Locator::Flow {
                    chapter: 99,
                    block: 0,
                    offset: 0,
                },
            )
            .unwrap();
        assert!(
            (r.progress(&book, end).unwrap() - 1.0).abs() < 1e-9,
            "{file}"
        );
        // Whole-book search finds the river (নদী) in more than one chapter.
        let mut s = BookSearch::new("নদী");
        let mut hits = Vec::new();
        loop {
            let b = s.next(&book, &mut r, 10);
            hits.extend(b.hits);
            if b.done {
                break;
            }
        }
        assert!(hits.len() >= 2, "{file}: {} hits", hits.len());
        let t = r.text_between(&book, hits[0].start, hits[0].end).unwrap();
        assert_eq!(t, "নদী", "{file}");
    }
}

#[test]
fn comics_are_page_books() {
    let book = akkhor::open(path("manga.cbz")).unwrap();
    assert_eq!(book.format(), Format::Cbz);
    assert!(book.is_paged() && book.flow().is_none());
    let info = book.info();
    assert_eq!(info.length, 3);
    assert!(info.right_to_left && !info.reflowable);
    assert_eq!(info.series.as_deref(), Some("অক্ষর কমিক 3"));
    assert!(info.to_json().contains("\"direction\":\"rtl\""));
    assert!(book.cover(100, 100).is_some());
    let mut r = Reading::new(&book);
    assert_eq!(
        r.at_progress(&book, 0.99).unwrap(),
        Locator::Page { page: 2, offset: 0 }
    );
    assert_eq!(
        r.clamp(&book, Locator::Page { page: 9, offset: 5 })
            .unwrap(),
        Locator::Page { page: 2, offset: 0 }
    );
    let mut s = BookSearch::new("x");
    assert!(s.next(&book, &mut r, 5).done);
}

#[test]
fn unsupported_containers_are_named() {
    for (bytes, name) in [
        (&b"Rar!\x1a\x07\x00..."[..], "CBR (RAR)"),
        (&b"7z\xbc\xaf\x27\x1c..."[..], "CB7 (7-Zip)"),
        (&b"\xeaDRMION\xee..."[..], "KFX (DRM)"),
        (&b"CONT\x02\x00..."[..], "KFX"),
        (&b"TPZ0..."[..], "Topaz"),
    ] {
        let r = akkhor::open_source(akkhor::Source::from_vec(bytes.to_vec()), Some("book.cbz"));
        assert!(
            matches!(r, Err(Error::Unsupported(n)) if n == name),
            "{name}"
        );
    }
    let _: Option<&Book> = None;
}
