//! Prints the horizontal gaps between glyphs per line, in ems, to study
//! word-gap thresholds. cargo run -p akkhor-pdf --example gaps -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let g = doc.glyphs(page).unwrap();
    let mut line: Vec<String> = Vec::new();
    let mut prev: Option<&akkhor_pdf::glyphs::PdfGlyph> = None;
    let mut pen = f32::MIN;
    for x in &g.glyphs {
        if let Some(p) = prev {
            if (x.origin.1 - p.origin.1).abs() > p.size * 0.5 || x.origin.0 + p.size < p.origin.0 {
                println!("{}", line.join(" "));
                line.clear();
                pen = f32::MIN;
            } else {
                let gap = (x.origin.0 - pen) / x.size;
                line.push(format!(
                    "{}{:.2}",
                    x.unicode
                        .as_deref()
                        .unwrap_or("?")
                        .chars()
                        .next()
                        .unwrap_or('?'),
                    gap
                ));
            }
        }
        pen = pen.max(x.origin.0 + x.advance);
        prev = Some(x);
    }
    println!("{}", line.join(" "));
}
