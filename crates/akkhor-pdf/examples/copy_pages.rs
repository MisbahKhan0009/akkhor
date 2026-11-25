//! Writes chosen pages of a PDF as a new PDF.
//!
//!   cargo run --release -p akkhor-pdf --features write --example copy_pages -- in.pdf out.pdf 1,3,5-7
//!
//! Pages are 1-based here, as people number them.

use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: copy_pages IN.pdf OUT.pdf PAGES (e.g. 1,3,5-7)");
        std::process::exit(2);
    }
    let mut pages = Vec::new();
    for part in args[3].split(',') {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let (a, b): (usize, usize) = (a.trim().parse().unwrap(), b.trim().parse().unwrap());
        pages.extend((a..=b).map(|p| p - 1));
    }
    let doc = PdfDocument::open(&Source::open(&args[1]).unwrap()).unwrap();
    let t = std::time::Instant::now();
    match doc.write_pages(&pages) {
        Ok(bytes) => {
            std::fs::write(&args[2], &bytes).unwrap();
            println!(
                "{} pages, {} bytes, {:.1} ms",
                pages.len(),
                bytes.len(),
                t.elapsed().as_secs_f64() * 1e3
            );
        }
        Err(e) => {
            println!("refused: {e}");
            std::process::exit(1);
        }
    }
}
