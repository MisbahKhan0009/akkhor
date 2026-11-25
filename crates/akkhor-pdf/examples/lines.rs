//! Prints each line of a page with its box, size and style, as page
//! structure sees it. cargo run -p akkhor-pdf --example lines -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let t = doc.page_text(page).unwrap();
    for (r, i) in t.lines.iter().zip(&t.line_info) {
        let s: String = t.text[r.clone()].chars().take(40).collect();
        println!(
            "x {:6.1}-{:6.1} base {:6.1} size {:5.2} {}{}{} {s}",
            i.rect.x0,
            i.rect.x1,
            i.baseline,
            i.size,
            if i.bold { "B" } else { "-" },
            if i.italic { "I" } else { "-" },
            if i.artifact { "A" } else { "-" },
        );
    }
}
