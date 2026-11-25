//! Prints the pictures, rules and filled boxes of a page, as page
//! structure sees them. cargo run -p akkhor-pdf --example shapes -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let s = doc.page_shapes(page).unwrap();
    let show =
        |r: &hayro::kurbo::Rect| format!("{:6.1},{:6.1} - {:6.1},{:6.1}", r.x0, r.y0, r.x1, r.y1);
    for r in &s.pictures {
        println!("picture {}", show(r));
    }
    for r in &s.fills {
        println!("fill    {}", show(r));
    }
    for r in &s.rules {
        println!("rule    {}", show(r));
    }
}
