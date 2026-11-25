//! Prints glyphs drawn more than once at nearly the same place (fake bold),
//! with their offset in ems. cargo run -p akkhor-pdf --example dups -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let g = doc.glyphs(page).unwrap();
    let mut n = 0;
    for (i, a) in g.glyphs.iter().enumerate() {
        for b in &g.glyphs[i.saturating_sub(40)..i] {
            if a.glyph_id == b.glyph_id && a.font == b.font {
                let dx = (a.origin.0 - b.origin.0) / a.size;
                let dy = (a.origin.1 - b.origin.1) / a.size;
                if dx.abs() < 0.2 && dy.abs() < 0.2 {
                    n += 1;
                    if n <= 12 {
                        println!(
                            "gid {} dx {dx:.3} dy {dy:.3} gap {}",
                            a.glyph_id,
                            i - g.glyphs.iter().position(|x| std::ptr::eq(x, b)).unwrap()
                        );
                    }
                }
            }
        }
    }
    println!("{n} near-duplicates of {}", g.glyphs.len());
}
