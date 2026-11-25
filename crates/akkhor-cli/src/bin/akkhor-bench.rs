//! `akkhor-bench`: Akkhor's side of the engine comparison
//! (`scripts/bench/compare_engines.py`). Each command prints one JSON
//! object; timings are measured inside the process, like the other
//! engines' library calls.
//!
//! ```text
//! akkhor-bench text    FILE                    all pages' text
//! akkhor-bench speed   FILE PAGE SCALE RUNS    open, first render, warm render
//! akkhor-bench mem     FILE PAGE SCALE         resident memory before and at peak
//! akkhor-bench survive FILE                    open, count, render and read page 1
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
        Some("survive") => {
            let outcome = (|| -> Result<(), String> {
                let pdf = open(&arg(1)?)?;
                if pdf.page_count() > 0 {
                    pdf.render_page(0, 0.5, &Theme::ORIGINAL)
                        .map_err(|e| e.to_string())?;
                    pdf.page_text(0).map_err(|e| e.to_string())?;
                }
                Ok(())
            })();
            Ok(match outcome {
                Ok(()) => "{\"ok\":true}".to_string(),
                Err(e) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)),
            })
        }
        _ => Err("usage: akkhor-bench text|speed|mem|survive FILE ...".into()),
    }
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
