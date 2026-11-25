//! Locators and progress (Phase 5.2): a stored place survives font size and
//! screen changes, progress and the scrubber are inverses, PDF places
//! convert between the page view and the reflow, and bad locators open
//! somewhere sensible.

use akkhor::flow::{FontSet, LayoutOptions, paginate};
use akkhor::reading::screen_of;
use akkhor::{Book, Locator, Reading};

fn path(rel: &str) -> String {
    format!("{}/../../tests/{rel}", env!("CARGO_MANIFEST_DIR"))
}

fn open(rel: &str) -> (Book, Reading) {
    let book = akkhor::open(path(rel)).unwrap();
    let reading = Reading::new(&book);
    (book, reading)
}

/// The word a locator points into, in the chapter's text.
fn word_at(r: &mut Reading, book: &Book, loc: Locator) -> String {
    let Locator::Flow {
        chapter,
        block,
        offset,
    } = loc
    else {
        panic!("flow locator expected")
    };
    let c = r.chapter(book, chapter).unwrap();
    c.document.blocks[block].text[offset..]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

#[test]
fn a_place_survives_font_size_and_screen_changes() {
    let (book, mut r) = open("fixtures/indic-sample.epub");
    let fonts = FontSet::bundled();
    // Store the place at the top of the second screen of chapter 0, laid
    // out small; reopen it laid out large and on a narrower screen.
    let small = LayoutOptions {
        width: 1080.0,
        height: 400.0,
        font_size: 40.0,
        ..Default::default()
    };
    let doc = r.chapter(&book, 0).unwrap().document.clone();
    let pages = paginate(&doc, &small, &fonts);
    assert!(pages.len() > 1);
    let (block, offset) = pages[1].start().unwrap();
    let loc = Locator::Flow {
        chapter: 0,
        block,
        offset,
    };
    let stored = loc.to_string();
    let word = word_at(&mut r, &book, loc);
    for (width, font_size) in [(720.0, 52.0), (1440.0, 24.0)] {
        let opts = LayoutOptions {
            width,
            height: 1280.0,
            font_size,
            ..Default::default()
        };
        let pages = paginate(&doc, &opts, &fonts);
        let back = Locator::parse(&stored).unwrap();
        let Locator::Flow { block, offset, .. } = back else {
            unreachable!()
        };
        let screen = screen_of(&pages, block, offset);
        // The screen shown holds the stored place.
        let start = pages[screen].start().unwrap();
        let next = pages.get(screen + 1).and_then(|p| p.start());
        assert!(start <= (block, offset));
        assert!(next.is_none_or(|n| (block, offset) < n));
        assert_eq!(word_at(&mut r, &book, back), word);
    }
}

#[test]
fn progress_and_scrubber_are_inverses() {
    for rel in [
        "fixtures/indic-sample.epub",
        "corpus/bn.txt",
        "corpus/books/pdf/bn.xelatex.two.pdf",
    ] {
        let (book, mut r) = open(rel);
        let mut last = -1.0;
        for k in 0..=20 {
            let f = f64::from(k) / 20.0;
            let loc = r.at_progress(&book, f).unwrap();
            let p = r.progress(&book, loc).unwrap();
            // Back to the same fraction (to a character), and monotonic.
            assert!((p - f).abs() < 0.01, "{rel}: {f} -> {loc} -> {p}");
            assert!(p >= last, "{rel}: progress went back at {f}");
            last = p;
        }
        assert_eq!(r.progress(&book, Locator::START).unwrap(), 0.0);
    }
}

#[test]
fn bad_locators_open_somewhere_sensible() {
    let (book, mut r) = open("fixtures/indic-sample.epub");
    let n = r.chapter_count();
    // Past the end of the book: its end.
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
    assert!(matches!(end, Locator::Flow { chapter, .. } if chapter == n - 1));
    assert!((r.progress(&book, end).unwrap() - 1.0).abs() < 1e-9);
    // An offset inside a Bangla letter lands on its start.
    let l = r
        .clamp(
            &book,
            Locator::Flow {
                chapter: 0,
                block: 0,
                offset: 1,
            },
        )
        .unwrap();
    assert_eq!(
        l,
        Locator::Flow {
            chapter: 0,
            block: 0,
            offset: 0
        }
    );
}

#[test]
fn pdf_places_convert_between_views() {
    let (book, mut r) = open("corpus/books/pdf/bn.chrome.two.pdf");
    let Book::Pdf(pdf) = &book else {
        unreachable!()
    };
    let text = pdf.page_text(1).unwrap().text;
    // A word of body text in the middle of page 2 (not the running header,
    // which the reflow leaves out).
    let mid = (0..=text.len() / 2)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap();
    let at = mid + text[mid..].find(' ').unwrap() + 1;
    let page = Locator::Page {
        page: 1,
        offset: at,
    };
    let flow = r.convert(&book, page).unwrap();
    assert!(matches!(flow, Locator::Flow { .. }));
    let back = r.convert(&book, flow).unwrap();
    assert_eq!(back, page);
    // Progress agrees roughly between the two views of one place.
    let a = r.progress(&book, page).unwrap();
    let b = r.progress(&book, flow).unwrap();
    assert!((a - b).abs() < 0.35, "{a} vs {b}");
}

#[test]
fn text_between_crosses_blocks_and_chapters() {
    let (book, mut r) = open("fixtures/indic-sample.epub");
    let start = Locator::Flow {
        chapter: 0,
        block: 1,
        offset: 0,
    };
    let end = Locator::Flow {
        chapter: 1,
        block: 0,
        offset: usize::MAX,
    };
    let t = r.text_between(&book, start, end).unwrap();
    assert!(t.starts_with("ভাষা আমাদের"), "{t}");
    assert!(t.lines().count() >= 4);
    // Reversed ends give the same text.
    assert_eq!(r.text_between(&book, end, start).unwrap(), t);
}

#[test]
fn sentences_for_read_aloud_follow_on() {
    let (book, mut r) = open("fixtures/indic-sample.epub");
    let s = r.sentences(&book, Locator::START, 6).unwrap();
    assert_eq!(s.len(), 6);
    assert_eq!(s[0].text, "প্রথম অধ্যায়: অক্ষরের কথা");
    assert_eq!(s[1].text, "ভাষা আমাদের পরিচয়ের সবচেয়ে গভীর চিহ্ন।");
    // Reading on from the end of the last one continues without a gap.
    let more = r.sentences(&book, s[5].end, 1).unwrap();
    assert!(more[0].start > s[5].start);
    // Each sentence's locators name its text.
    for x in &s {
        assert_eq!(r.text_between(&book, x.start, x.end).unwrap(), x.text);
    }
}
