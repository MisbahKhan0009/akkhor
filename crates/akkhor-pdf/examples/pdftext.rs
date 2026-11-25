//! Prints a PDF page's text: raw text layer, repaired text and font info.
//! cargo run -p akkhor-pdf --example pdftext -- file.pdf [page]
use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file");
    let page: usize = args.next().map(|p| p.parse().unwrap()).unwrap_or(0);
    if std::env::var("NONAMES").is_ok() {
        akkhor_pdf::reference::set_glyph_names(false);
    }
    for f in ["HindSiliguri-Regular.ttf", "Mukta-Regular.ttf"] {
        if let Ok(d) = std::fs::read(format!("tests/fonts/{f}")) {
            akkhor_pdf::register_reference_font(std::sync::Arc::new(d));
        }
    }
    let pdf = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    let glyphs = pdf.glyphs(page).unwrap();
    for f in &glyphs.fonts {
        let rec = f.data.as_ref().and_then(akkhor_pdf::reference::recover);
        println!(
            "font {:?} embedded={} recovered={:?}",
            f.name,
            f.data.is_some(),
            rec.map(|r| (r.map.len(), r.indic))
        );
    }
    if let Ok(pat) = std::env::var("DUMP") {
        for f in &glyphs.fonts {
            if let Some(r) = f.data.as_ref().and_then(akkhor_pdf::reference::recover) {
                let mut v: Vec<_> = r
                    .map
                    .iter()
                    .filter(|(_, t)| t.text.contains(pat.as_str()))
                    .collect();
                v.sort_by_key(|(g, _)| **g);
                for (g, t) in v {
                    println!("  gid {g:>4} {:?} {:?}", t.text, t.role);
                }
            }
        }
    }
    if std::env::var("MISSING").is_ok() {
        let recs: Vec<_> = glyphs
            .fonts
            .iter()
            .map(|f| f.data.as_ref().and_then(akkhor_pdf::reference::recover))
            .collect();
        let mut seen = std::collections::BTreeMap::new();
        for g in &glyphs.glyphs {
            let r = recs[g.font].as_ref();
            if r.is_some_and(|r| !r.map.contains_key(&g.glyph_id)) {
                *seen.entry((g.glyph_id, g.unicode.clone())).or_insert(0) += 1;
            }
        }
        for ((gid, u), n) in seen {
            println!("  missing gid {gid:>4} x{n:<3} tu {u:?}");
        }
    }
    if std::env::var("GLYPHS").is_ok() {
        let recs: Vec<_> = glyphs
            .fonts
            .iter()
            .map(|f| f.data.as_ref().and_then(akkhor_pdf::reference::recover))
            .collect();
        for g in &glyphs.glyphs {
            let r = recs[g.font].as_ref().and_then(|r| r.map.get(&g.glyph_id));
            println!(
                "  gid {:>4} x {:>6.1} y {:>6.1} adv {:>5.1} tu {:?} rec {:?}",
                g.glyph_id,
                g.origin.0,
                g.origin.1,
                g.advance,
                g.unicode,
                r.map(|r| (&r.text, r.role))
            );
        }
    }
    println!("--- raw\n{}", pdf.page_text_raw(page).unwrap().text);
    println!("--- akkhor\n{}", pdf.page_text(page).unwrap().text);
}
// (debug helper appended)
