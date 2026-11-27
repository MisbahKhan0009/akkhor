//! `akkhor-bench`: Akkhor's side of the engine comparison
//! (`scripts/bench/compare_engines.py`). Each command prints one JSON
//! object; timings are measured inside the process, like the other
//! engines' library calls.
//!
//! ```text
//! akkhor-bench text    FILE                    all pages' text
//! akkhor-bench reflow  FILE                    every reflow chapter's blocks
//! akkhor-bench speed   FILE PAGE SCALE RUNS    open, first render, warm render
//! akkhor-bench mem     FILE PAGE SCALE         resident memory before and at peak
//! akkhor-bench survive FILE                    open, count, render and read page 1
//! akkhor-bench library FILE...                 library cards for many books in one process
//! akkhor-bench phone   FILE                    what the app does, timed (any format;
//!                                              scripts/phone/README.md)
//! akkhor-bench tiles   FILE PAGE SCALE         every 512×512 tile of a zoomed page
//! akkhor-bench session FILE PAGES SCALE        render pages 1..PAGES in one document:
//!                                              memory after every 10 pages (all
//!                                              resident, and heap only: the rest
//!                                              is the file's pages, which the OS
//!                                              can drop at any time)
//! ```
//!
//! `AKKHOR_REFERENCE_FONTS=DIR` registers every font in DIR as a PDF
//! reference font, as the Android app does with the phone's fonts.

use std::time::Instant;

use akkhor::pdf::PdfDocument;
use akkhor::{Source, Theme};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn status_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

/// glibc keeps freed memory for reuse; ask it to hand it back so resident
/// memory shows what is really in use (no-op elsewhere).
fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        unsafe {
            malloc_trim(0);
        }
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list(v: &[f64]) -> String {
    let items: Vec<String> = v.iter().map(|x| format!("{x:.4}")).collect();
    format!("[{}]", items.join(","))
}

fn open(path: &str) -> Result<PdfDocument, String> {
    let src = Source::open(path).map_err(|e| e.to_string())?;
    PdfDocument::open(&src).map_err(|e| e.to_string())
}

fn register_fonts() {
    let Ok(dir) = std::env::var("AKKHOR_REFERENCE_FONTS") else {
        return;
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "ttf" || e == "otf")
            && let Ok(src) = Source::open(&p)
        {
            akkhor::pdf::register_reference_font(src.shared());
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let arg = |i: usize| args.get(i).cloned().ok_or("missing argument");
    match args.first().map(String::as_str) {
        Some("text") => {
            let t = Instant::now();
            let pdf = open(&arg(1)?)?;
            let pages: Vec<String> = (0..pdf.page_count())
                .map(|i| pdf.page_text(i).map(|t| t.text).unwrap_or_default())
                .collect();
            let total = ms(t);
            let pages: Vec<String> = pages.iter().map(|p| json_str(p)).collect();
            Ok(format!(
                "{{\"ms\":{total:.4},\"pages\":[{}]}}",
                pages.join(",")
            ))
        }
        Some("reflow") => {
            let t = Instant::now();
            let pdf = open(&arg(1)?)?;
            let mut blocks = Vec::new();
            let mut chapters = Vec::new();
            for range in pdf.reflow_chapters() {
                let tc = Instant::now();
                let r = pdf.reflow(range.clone()).map_err(|e| e.to_string())?;
                chapters.push(format!("[{},{},{:.4}]", range.start, range.end, ms(tc)));
                for (i, b) in r.document.blocks.iter().enumerate() {
                    let (kind, level) = match b.kind {
                        akkhor::flow::BlockKind::Heading(l) => ("heading", l),
                        akkhor::flow::BlockKind::Image => ("image", 0),
                        akkhor::flow::BlockKind::Quote => ("card", 0),
                        _ if b.style.font_scale.is_some_and(|s| s < 1.0) => ("footnote", 0),
                        _ => ("paragraph", 0),
                    };
                    blocks.push(format!(
                        "{{\"kind\":\"{kind}\",\"level\":{level},\"page\":{},\"text\":{}}}",
                        r.block_page(i).unwrap_or(0),
                        json_str(&b.text)
                    ));
                }
            }
            Ok(format!(
                "{{\"ms\":{:.4},\"chapters\":[{}],\"blocks\":[{}]}}",
                ms(t),
                chapters.join(","),
                blocks.join(",")
            ))
        }
        Some("speed") => {
            let (file, page, scale, runs) = (
                arg(1)?,
                arg(2)?.parse::<usize>().map_err(|_| "page")?,
                arg(3)?.parse::<f32>().map_err(|_| "scale")?,
                arg(4)?.parse::<usize>().map_err(|_| "runs")?,
            );
            let (mut opens, mut firsts, mut warms) = (vec![], vec![], vec![]);
            for _ in 0..runs {
                let t = Instant::now();
                let pdf = open(&file)?;
                opens.push(ms(t));
                let t = Instant::now();
                pdf.render_page(page, scale, &Theme::ORIGINAL)
                    .map_err(|e| e.to_string())?;
                firsts.push(ms(t));
                let t = Instant::now();
                pdf.render_page(page, scale, &Theme::ORIGINAL)
                    .map_err(|e| e.to_string())?;
                warms.push(ms(t));
            }
            Ok(format!(
                "{{\"open_ms\":{},\"first_render_ms\":{},\"warm_render_ms\":{}}}",
                list(&opens),
                list(&firsts),
                list(&warms)
            ))
        }
        Some("mem") => {
            // Reset the peak so it excludes start-up (Linux).
            let _ = std::fs::write("/proc/self/clear_refs", "5");
            let before = status_kb("VmRSS:");
            let pdf = open(&arg(1)?)?;
            let page = arg(2)?.parse::<usize>().map_err(|_| "page")?;
            let scale = arg(3)?.parse::<f32>().map_err(|_| "scale")?;
            let rendered = pdf
                .render_page(page, scale, &Theme::ORIGINAL)
                .map_err(|e| e.to_string())?;
            let peak = status_kb("VmHWM:");
            drop(rendered);
            Ok(format!("{{\"rss_before_kb\":{before},\"peak_kb\":{peak}}}"))
        }
        Some("tiles") => {
            let (file, page, scale) = (
                arg(1)?,
                arg(2)?.parse::<usize>().map_err(|_| "page")?,
                arg(3)?.parse::<f32>().map_err(|_| "scale")?,
            );
            let pdf = open(&file)?;
            let (w, h) = pdf.page_pixels(page, scale).map_err(|e| e.to_string())?;
            let mut times = vec![];
            let mut buf = vec![0u8; 512 * 512 * 4];
            for y in (0..h).step_by(512) {
                for x in (0..w).step_by(512) {
                    let tile = akkhor::Tile::new(x, y, 512.min(w - x), 512.min(h - y));
                    let t = Instant::now();
                    pdf.render_tile(page, scale, tile, &Theme::ORIGINAL, &mut buf)
                        .map_err(|e| e.to_string())?;
                    times.push(ms(t));
                }
            }
            Ok(format!("{{\"tile_ms\":{}}}", list(&times)))
        }
        Some("session") => {
            let (file, pages, scale) = (
                arg(1)?,
                arg(2)?.parse::<usize>().map_err(|_| "pages")?,
                arg(3)?.parse::<f32>().map_err(|_| "scale")?,
            );
            let _ = std::fs::write("/proc/self/clear_refs", "5");
            let before = status_kb("VmRSS:");
            let anon_before = status_kb("RssAnon:");
            let pdf = open(&file)?;
            let mut rss = vec![];
            let mut anon = vec![];
            let t = Instant::now();
            for p in 0..pages.min(pdf.page_count()) {
                pdf.render_page(p, scale, &Theme::ORIGINAL)
                    .map_err(|e| e.to_string())?;
                if (p + 1) % 10 == 0 {
                    release_freed_memory();
                    rss.push((status_kb("VmRSS:").saturating_sub(before)) as f64 / 1024.0);
                    anon.push((status_kb("RssAnon:").saturating_sub(anon_before)) as f64 / 1024.0);
                }
            }
            let total = ms(t);
            let trimmed = pdf.trimmed().map_err(|e| e.to_string())?;
            drop(pdf);
            release_freed_memory();
            let after_trim = status_kb("RssAnon:").saturating_sub(anon_before) as f64 / 1024.0;
            drop(trimmed);
            Ok(format!(
                "{{\"total_ms\":{:.3},\"rss_added_mb\":{},\"heap_added_mb\":{},\"heap_after_trim_mb\":{after_trim:.2},\"peak_added_mb\":{:.2}}}",
                total,
                list(&rss),
                list(&anon),
                status_kb("VmHWM:").saturating_sub(before) as f64 / 1024.0
            ))
        }
        Some("phone") => phone(&arg(1)?),
        Some("library") => library(&args[1..]),
        Some("survive") => {
            let outcome = (|| -> Result<(), String> {
                let pdf = open(&arg(1)?)?;
                // Opened means a page shown (the same rule for every engine).
                if pdf.page_count() == 0 {
                    return Err("no pages".into());
                }
                pdf.render_page(0, 0.5, &Theme::ORIGINAL)
                    .map_err(|e| e.to_string())?;
                pdf.page_text(0).map_err(|e| e.to_string())?;
                Ok(())
            })();
            Ok(match outcome {
                Ok(()) => "{\"ok\":true}".to_string(),
                Err(e) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)),
            })
        }
        _ => Err(
            "usage: akkhor-bench text|reflow|speed|mem|tiles|session|survive|phone FILE ...".into(),
        ),
    }
}

/// What the app does with a book, timed, on a phone-sized screen (1080 ×
/// 2200 px, 42 px text): open, library card (info and cover), the first
/// screen (a PDF page fitted to the width, or the first chapter laid out
/// and drawn), a redraw, the first chapter as reflowed text, a search,
/// and memory.
fn phone(file: &str) -> Result<String, String> {
    use akkhor::flow::{FontSet, GlyphCache, LayoutOptions, paginate, render_page};
    let (w, h, size) = (1080u32, 2200u32, 42.0f32);
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let before = status_kb("VmRSS:");
    let mut out = vec![format!("\"file\":{}", json_str(file))];
    let time = |out: &mut Vec<String>, name: &str, t: Instant| {
        out.push(format!("\"{name}_ms\":{:.2}", ms(t)))
    };

    let t = Instant::now();
    let book = akkhor::open(file).map_err(|e| e.to_string())?;
    time(&mut out, "open", t);
    let t = Instant::now();
    let info = book.info();
    time(&mut out, "info", t);
    let t = Instant::now();
    let cover = book.cover(300, 450);
    time(&mut out, "cover", t);
    let fonts = FontSet::bundled();
    let opts = LayoutOptions {
        width: w as f32,
        height: h as f32,
        font_size: size,
        margin_x: w as f32 * 0.067,
        ..Default::default()
    };
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    if let akkhor::Book::Pdf(pdf) = &book {
        let scale = w as f32 / pdf.page_size(0).map_err(|e| e.to_string())?.0;
        let t = Instant::now();
        pdf.render_page(0, scale, &Theme::ORIGINAL)
            .map_err(|e| e.to_string())?;
        time(&mut out, "first_page", t);
        let t = Instant::now();
        pdf.render_page(0, scale, &Theme::ORIGINAL)
            .map_err(|e| e.to_string())?;
        time(&mut out, "redraw", t);
        let t = Instant::now();
        let _ = pdf.page_text(0);
        time(&mut out, "page_text", t);
    }
    // A comic's page view: the page fitted to the width, then a redraw.
    if let akkhor::Book::Cbz(comic) = &book {
        let scale = w as f32 / comic.page_size(0).map_err(|e| e.to_string())?.0;
        let t = Instant::now();
        comic
            .render_page(0, scale, &Theme::ORIGINAL)
            .map_err(|e| e.to_string())?;
        time(&mut out, "first_page", t);
        let t = Instant::now();
        comic
            .render_page(0, scale, &Theme::ORIGINAL)
            .map_err(|e| e.to_string())?;
        time(&mut out, "redraw", t);
    }
    let mut reading = akkhor::Reading::new(&book);
    if reading.chapter_count() > 0 {
        let t = Instant::now();
        let doc = reading
            .chapter(&book, 0)
            .map_err(|e| e.to_string())?
            .document
            .clone();
        time(&mut out, "chapter", t);
        let t = Instant::now();
        let pages = paginate(&doc, &opts, &fonts);
        time(&mut out, "layout", t);
        out.push(format!("\"screens\":{}", pages.len()));
        let mut glyphs = GlyphCache::default();
        if let Some(first) = pages.first() {
            let t = Instant::now();
            render_page(
                first,
                &fonts,
                None,
                &mut glyphs,
                &Theme::PAPER,
                w,
                h,
                &mut pixels,
            )
            .map_err(|e| e.to_string())?;
            time(&mut out, "draw_screen", t);
            let t = Instant::now();
            render_page(
                first,
                &fonts,
                None,
                &mut glyphs,
                &Theme::PAPER,
                w,
                h,
                &mut pixels,
            )
            .map_err(|e| e.to_string())?;
            time(&mut out, "redraw_screen", t);
        }
    }
    let t = Instant::now();
    let mut search = akkhor::search::BookSearch::new("এবং");
    let mut hits = 0;
    loop {
        let batch = search.next(&book, &mut reading, 24);
        hits += batch.hits.len();
        if batch.done {
            break;
        }
    }
    time(&mut out, "search", t);
    out.push(format!("\"hits\":{hits}"));
    out.push(format!("\"has_cover\":{}", cover.is_some()));
    out.push(format!("\"length\":{}", info.length));
    out.push(format!(
        "\"peak_added_mb\":{:.1}",
        status_kb("VmHWM:").saturating_sub(before) as f64 / 1024.0
    ));
    Ok(format!("{{{}}}", out.join(",")))
}

/// A library scan in one process, as the app does it: open, info and a
/// cover for each file, twice. The first file pays the one-time costs.
fn library(files: &[String]) -> Result<String, String> {
    let mut rows = Vec::new();
    for pass in ["first", "again"] {
        for f in files {
            let t = Instant::now();
            let book = akkhor::open(f).map_err(|e| e.to_string())?;
            let open = ms(t);
            let t = Instant::now();
            let info = book.info();
            let info_ms = ms(t);
            let t = Instant::now();
            let cover = book.cover(300, 450);
            let cover_ms = ms(t);
            let name = std::path::Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            rows.push(format!(
                "{{\"file\":{},\"pass\":\"{pass}\",\"open_ms\":{open:.2},\"info_ms\":{info_ms:.2},\"cover_ms\":{cover_ms:.2},\"has_cover\":{},\"text_quality\":{}}}",
                json_str(&name),
                cover.is_some(),
                info.text_quality.map_or("null".into(), |q| format!("{q:.3}"))
            ));
        }
    }
    Ok(rows.join("\n"))
}

fn main() -> std::process::ExitCode {
    register_fonts();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(json) => {
            println!("{json}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{{\"error\":{}}}", json_str(&e));
            std::process::ExitCode::FAILURE
        }
    }
}
