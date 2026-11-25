//! Runs a file through the steps of the PDF fuzz targets, timing each, to
//! find which one is slow or fails on an input the fuzzer saved.
//!
//!   cargo run --release -p akkhor-pdf --features write --example replay -- fuzz/artifacts/pdf/timeout-...

use std::time::Instant;

use akkhor_core::{Source, Theme};
use akkhor_pdf::PdfDocument;

fn step<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let v = f();
    println!("{name:>24}: {:8.1} ms", t.elapsed().as_secs_f64() * 1e3);
    v
}

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let data = std::fs::read(path).unwrap();
    let Ok(doc) = step("open", || PdfDocument::open(&Source::from_vec(data))) else {
        println!("does not open");
        return;
    };
    step("info", || doc.info());
    step("outline", || doc.outline());
    let n = doc.page_count();
    println!("{n} pages");
    for i in 0..n.min(2) {
        let _ = step(&format!("page_text {i}"), || {
            doc.page_text(i).map(|t| t.text.len())
        });
        step(&format!("thumbnail {i}"), || {
            doc.thumbnail(i, 48, &Theme::ORIGINAL).is_ok()
        });
    }
    step("text_quality", || doc.text_quality(0..n.min(2)));
    let chapters = step("reflow_chapters", || doc.reflow_chapters());
    if let Some(c) = chapters.first() {
        step("reflow first", || {
            doc.reflow(c.start..c.end.min(c.start + 3)).is_ok()
        });
    }
    if n > 0 {
        #[cfg(feature = "write")]
        {
            let r = step("write_pages", || doc.write_pages(&[n - 1, 0]));
            if let Ok(bytes) = r {
                let copy = step("open copy", || {
                    PdfDocument::open(&Source::from_vec(bytes)).unwrap()
                });
                step("copy page_text 0", || copy.page_text(0).is_ok());
            }
        }
    }
}
