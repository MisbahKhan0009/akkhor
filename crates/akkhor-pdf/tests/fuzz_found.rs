//! Inputs the fuzzer found (tests/corpus/fuzz): each must go through the
//! steps of the fuzz targets quickly, without panicking.

use std::path::Path;
use std::time::{Duration, Instant};

use akkhor_core::{Source, Theme};
use akkhor_pdf::PdfDocument;

/// The steps of the `pdf`, `pdf_reflow` and `pdf_write` targets.
fn run(data: Vec<u8>) {
    let Ok(doc) = PdfDocument::open(&Source::from_vec(data)) else {
        return;
    };
    let _ = doc.info();
    let _ = doc.outline();
    let n = doc.page_count();
    for i in 0..n.min(2) {
        let _ = doc.page_text(i);
        let _ = doc.thumbnail(i, 48, &Theme::ORIGINAL);
    }
    let _ = doc.text_quality(0..n.min(2));
    if let Some(c) = doc.reflow_chapters().first() {
        let _ = doc.reflow(c.start..c.end.min(c.start + 3));
    }
    #[cfg(feature = "write")]
    if n > 0
        && let Ok(bytes) = doc.write_pages(&[n - 1, 0])
    {
        let copy = PdfDocument::open(&Source::from_vec(bytes)).unwrap();
        let _ = copy.page_text(0);
    }
}

#[test]
fn inputs_found_by_fuzzing_finish_quickly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/fuzz");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "bin") {
            continue;
        }
        let t = Instant::now();
        run(std::fs::read(&p).unwrap());
        // Unoptimised builds are slow; before the fixes these took minutes.
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "{}: {:?}",
            p.display(),
            t.elapsed()
        );
        n += 1;
    }
    assert!(n >= 2);
}

#[test]
fn an_unterminated_nest_of_arrays_fails_at_once() {
    // A page whose /MediaBox opens 40 arrays and never closes them.
    let nest = "[".repeat(40);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox {nest} 0 0 612 792 >>"),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let start = pdf.len();
    pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f\r\n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n").as_bytes(),
    );
    let t = Instant::now();
    run(pdf);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}
