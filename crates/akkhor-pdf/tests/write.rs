//! Pages copied as a new PDF read back the same in Akkhor: same text,
//! same pixels, for every PDF in the test corpus. MuPDF, PDFium, Poppler
//! and qpdf are checked by `scripts/bench/check_page_copies.py`.
#![cfg(feature = "write")]

use std::path::{Path, PathBuf};

use akkhor_core::{Error, Source, Theme};
use akkhor_pdf::PdfDocument;

fn pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            pdfs(&p, out);
        } else if p.extension().is_some_and(|x| x == "pdf") {
            out.push(p);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn copied_pages_read_and_draw_the_same() {
    let mut files = Vec::new();
    pdfs(&root().join("tests"), &mut files);
    files.sort();
    let mut copied = 0;
    for f in &files {
        let Ok(doc) = PdfDocument::open(&Source::open(f).unwrap()) else {
            continue;
        };
        let n = doc.page_count();
        if n == 0 || doc.page_text(0).is_err() {
            continue;
        }
        let chosen: Vec<usize> = if n <= 4 {
            (0..n).rev().collect()
        } else {
            vec![n - 1, 0]
        };
        let bytes = match doc.write_pages(&chosen) {
            Ok(b) => b,
            Err(Error::Protected) => continue,
            Err(e) => panic!("{}: {e}", f.display()),
        };
        let copy = PdfDocument::open(&Source::from_vec(bytes)).unwrap();
        assert_eq!(copy.page_count(), chosen.len(), "{}", f.display());
        for (c, &o) in chosen.iter().enumerate() {
            let name = format!("{} page {}", f.display(), o + 1);
            assert_eq!(
                copy.page_size(c).unwrap(),
                doc.page_size(o).unwrap(),
                "{name}"
            );
            assert_eq!(
                copy.page_text(c).unwrap().text,
                doc.page_text(o).unwrap().text,
                "{name}"
            );
            let a = doc.render_page(o, 0.5, &Theme::ORIGINAL).unwrap();
            let b = copy.render_page(c, 0.5, &Theme::ORIGINAL).unwrap();
            assert!(a.pixels == b.pixels, "{name}: pixels differ");
        }
        copied += 1;
    }
    assert!(copied >= 70, "only {copied} files copied");
}

#[test]
fn encrypted_files_and_bad_pages_are_refused() {
    let open = |p: &str| PdfDocument::open(&Source::open(root().join(p)).unwrap()).unwrap();
    let locked = open("tests/corpus/write/encrypted.pdf");
    assert!(locked.page_text(0).is_ok(), "opens with the empty password");
    assert!(matches!(locked.write_pages(&[0]), Err(Error::Protected)));
    let book = open("tests/corpus/write/objstreams.pdf");
    assert!(matches!(book.write_pages(&[]), Err(Error::OutOfRange)));
    assert!(matches!(book.write_pages(&[2]), Err(Error::OutOfRange)));
    // Repeats are allowed: the same page twice.
    let twice = book.write_pages(&[1, 1]).unwrap();
    let copy = PdfDocument::open(&Source::from_vec(twice)).unwrap();
    assert_eq!(
        copy.page_text(0).unwrap().text,
        copy.page_text(1).unwrap().text
    );
}

#[test]
fn inherited_attributes_and_form_fields_come_along() {
    let open = |p: &str| PdfDocument::open(&Source::open(root().join(p)).unwrap()).unwrap();
    let doc = open("tests/corpus/write/inherited.pdf");
    let copy = PdfDocument::open(&Source::from_vec(doc.write_pages(&[2, 0]).unwrap())).unwrap();
    // Rotated by the page tree: a 300 x 300 and a 400 x 800 page turned.
    assert_eq!(copy.page_size(0).unwrap(), (300.0, 300.0));
    assert_eq!(copy.page_size(1).unwrap(), (800.0, 400.0));
    let text: String = copy.page_text(1).unwrap().text.split_whitespace().collect();
    assert!(text.contains("Pageone"), "{text}");

    let doc = open("tests/corpus/write/annots.pdf");
    let bytes = doc.write_pages(&[1]).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("/AcroForm << /Fields ["),
        "the form lists the field"
    );
    assert!(
        text.contains("/FT /Tx"),
        "the field type comes from the parent"
    );
    assert!(!text.contains("/Subtype /Link"), "links are left out");
    // The whole copy is a page and what it uses: no other pages.
    assert_eq!(text.matches("/Type /Page ").count(), 1);
}
