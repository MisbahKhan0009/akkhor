//! Prints the Type 3 glyph shapes of a page and their best matches in the
//! bundled and registered fonts. cargo run -p akkhor-pdf --example type3 -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    for f in [
        "fonts/NotoSerifBengali-Regular.ttf",
        "fonts/NotoSerifDevanagari-Regular.ttf",
    ] {
        akkhor_pdf::register_reference_font(std::sync::Arc::new(std::fs::read(f).unwrap()));
    }
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let g = doc.glyphs(page).unwrap();
    println!("{} type 3 shapes", g.type3_shapes.len());
    for (i, (s, adv)) in g.type3_shapes.iter().enumerate().take(6) {
        let cells: u32 = s.0.iter().map(|r| r.count_ones()).sum();
        println!("shape {i}: advance {adv:.0}, {cells} cells");
        for row in s.0.iter().rev() {
            let line: String = (0..64)
                .map(|c| if row >> c & 1 == 1 { '#' } else { '.' })
                .collect();
            println!("  {line}");
        }
    }
    let texts = akkhor_pdf::reference::recognise(&g.type3_shapes);
    println!(
        "{:?}",
        texts
            .iter()
            .map(|t| t.as_ref().map(|t| (t.text.text.clone(), t.bolder)))
            .collect::<Vec<_>>()
    );
}
