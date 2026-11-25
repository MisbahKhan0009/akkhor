//! Prints the text quality (0 to 1) of the first pages of PDFs.
//! cargo run -p akkhor-pdf --example quality -- file.pdf... (REFDIR=dir to register fonts)
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    if let Ok(dir) = std::env::var("REFDIR") {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path()
                .extension()
                .is_some_and(|x| x == "ttf" || x == "otf")
            {
                akkhor_pdf::register_reference_font(std::sync::Arc::new(
                    std::fs::read(e.path()).unwrap(),
                ));
            }
        }
    }
    for path in std::env::args().skip(1) {
        let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
        let t = std::time::Instant::now();
        let q = doc.text_quality(0..3);
        println!("{q:?} {:.0} ms {path}", t.elapsed().as_secs_f64() * 1e3);
    }
}
