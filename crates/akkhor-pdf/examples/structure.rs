//! Prints the blocks page structure finds on a page (kind, box, text).
//! cargo run -p akkhor-pdf --example structure -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;
use akkhor_pdf::structure::{PageInput, analyse};
use hayro::kurbo::Rect;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    for dir in ["fonts", "tests/fonts"] {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "ttf") {
                akkhor_pdf::register_reference_font(std::sync::Arc::new(
                    std::fs::read(e.path()).unwrap(),
                ));
            }
        }
    }
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let text = doc.page_text(page).unwrap();
    let shapes = doc.page_shapes(page).unwrap();
    let (w, h) = doc.page_size(page).unwrap();
    let input = [PageInput {
        text: &text,
        images: &shapes.pictures,
        rules: &shapes.rules,
        fills: &shapes.fills,
        page: Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
    }];
    for b in &analyse(&input)[0].blocks {
        let t: String = b
            .lines
            .iter()
            .map(|&l| &text.text[text.lines[l].clone()])
            .collect::<Vec<_>>()
            .join(" | ");
        println!(
            "{:?}{} r{} y {:.0}-{:.0}: {}",
            b.kind,
            if b.boxed { " boxed" } else { "" },
            b.region,
            b.rect.y0,
            b.rect.y1,
            t.chars().take(70).collect::<String>()
        );
    }
}
