//! Shared corpus runner for the accuracy tests.

#![allow(dead_code)]

use std::sync::Arc;

use akkhor_core::Source;
use akkhor_pdf::PdfDocument;

pub fn corpus(name: &str) -> String {
    format!("{}/../../tests/corpus/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Folded, whitespace-free characters: what a reader would compare.
pub fn normal(s: &str) -> Vec<char> {
    akkhor_flow::search::fold(s)
        .text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + (ca != cb) as usize)
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn cer(truth: &str, got: &str) -> f64 {
    let (t, g) = (normal(truth), normal(got));
    edit_distance(&t, &g) as f64 / t.len().max(1) as f64
}

pub fn document_text(pdf: &PdfDocument, raw: bool) -> String {
    (0..pdf.page_count())
        .map(|i| {
            if raw {
                pdf.page_text_raw(i)
            } else {
                pdf.page_text(i)
            }
            .unwrap()
            .text
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Registers the corpus fonts as reference fonts: the extra designs in
/// tests/fonts, and the Noto ones (also compiled in with `bundled-fonts`).
pub fn register_test_fonts() {
    for f in [
        "../fonts/HindSiliguri-Regular.ttf",
        "../fonts/Mukta-Regular.ttf",
        "../../fonts/NotoSerifBengali-Regular.ttf",
        "../../fonts/NotoSerifDevanagari-Regular.ttf",
        "../../fonts/NotoSerif-Regular.ttf",
    ] {
        let data = std::fs::read(corpus(f)).unwrap();
        akkhor_pdf::register_reference_font(Arc::new(data));
    }
}

/// One row per corpus PDF: (file, CER of the PDF's own text, CER of
/// Akkhor's text).
pub fn run_corpus() -> Vec<(String, f64, f64)> {
    let mut rows = Vec::new();
    for stem in ["bn", "hi", "bn-conjuncts", "hi-conjuncts"] {
        let truth = std::fs::read_to_string(corpus(&format!("{stem}.txt"))).unwrap();
        let design = if stem.starts_with("bn") {
            "hind"
        } else {
            "mukta"
        };
        for base in [stem.to_string(), format!("{stem}.{design}")] {
            for producer in ["chrome", "libreoffice"] {
                for variant in ["", ".raw"] {
                    let file = format!("pdf/{base}.{producer}{variant}.pdf");
                    let pdf = PdfDocument::open(&Source::open(corpus(&file)).unwrap()).unwrap();
                    let before = cer(&truth, &document_text(&pdf, true));
                    let after = cer(&truth, &document_text(&pdf, false));
                    rows.push((file, before, after));
                }
            }
        }
    }
    // Set D: bn.txt typed in Bijoy, in a font named SutonnyMJ.
    let truth = std::fs::read_to_string(corpus("bn.txt")).unwrap();
    for producer in ["chrome", "libreoffice"] {
        for variant in ["", ".raw"] {
            let file = format!("pdf/bn.bijoy.{producer}{variant}.pdf");
            let pdf = PdfDocument::open(&Source::open(corpus(&file)).unwrap()).unwrap();
            let before = cer(&truth, &document_text(&pdf, true));
            let after = cer(&truth, &document_text(&pdf, false));
            rows.push((file, before, after));
        }
    }
    println!("{:<48} {:>10} {:>10}", "file", "per glyph", "Akkhor");
    for (f, b, a) in &rows {
        println!("{f:<48} {:>9.1}% {:>9.1}%", b * 100.0, a * 100.0);
    }
    rows
}

/// Targets (docs/phases/phase-2.md): under 2% with the PDF's text layer,
/// under 5% without it.
pub fn assert_targets(rows: &[(String, f64, f64)]) {
    for (f, _, a) in rows {
        let limit = if f.contains(".raw") { 0.05 } else { 0.02 };
        assert!(
            *a < limit,
            "{f}: {:.1}% is over {:.0}%",
            a * 100.0,
            limit * 100.0
        );
    }
}
