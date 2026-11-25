//! Times page structure and reflow alone, with the page text already
//! built, for every 16-page chapter of a PDF.
//! cargo run --release -p akkhor-pdf --example reflowtime -- file.pdf
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    if let Ok(dir) = std::env::var("REFDIR") {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            akkhor_pdf::register_reference_font(std::sync::Arc::new(
                std::fs::read(e.path()).unwrap(),
            ));
        }
    }
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let t = Instant::now();
    let texts: Vec<_> = (0..doc.page_count())
        .map(|i| doc.page_text(i).unwrap())
        .collect();
    println!(
        "page text, {} pages: {:.1} ms",
        texts.len(),
        t.elapsed().as_secs_f64() * 1e3
    );
    let mut worst: f64 = 0.0;
    let mut total = 0.0;
    for range in doc.reflow_chapters() {
        let shapes = akkhor_pdf::reflow::PageShapes::default();
        let pages: Vec<_> = range
            .clone()
            .map(|i| akkhor_pdf::reflow::ReflowPage {
                text: &texts[i],
                shapes: &shapes,
                size: doc.page_size(i).unwrap(),
            })
            .collect();
        let refs: Vec<_> = texts[range.clone()].iter().collect();
        let t = Instant::now();
        let r = akkhor_pdf::reflow::reflow(range.start, &pages, 0..pages.len());
        let ms = t.elapsed().as_secs_f64() * 1e3;
        worst = worst.max(ms);
        total += ms;
        assert!(!r.document.blocks.is_empty() || refs.iter().all(|t| t.text.trim().is_empty()));
    }
    println!("structure + reflow: total {total:.1} ms, worst chapter {worst:.2} ms");
}
