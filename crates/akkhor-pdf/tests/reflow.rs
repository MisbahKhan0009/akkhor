//! Reflow on the book-layout corpus (tests/corpus/books): reading order,
//! paragraph and heading boundaries, running headers and page numbers, line-
//! end hyphens and footnotes, against the ground truth written with each
//! PDF. Targets from docs/phases/phase-4.md (4.4).

mod common;

use akkhor_core::Source;
use akkhor_flow::BlockKind as Kind;
use akkhor_pdf::PdfDocument;
use common::*;

/// (kind, level, text) of each truth block, in order. The truth files are
/// written by scripts/make_books.py: one `"kind"`, an optional `"level"`
/// and one `"text"` per block, texts without quotes or backslashes.
fn truth_blocks(json: &str) -> Vec<(String, u8, String)> {
    let body = json.split("\"blocks\"").nth(1).unwrap_or("");
    let body = body.split("\"furniture\"").next().unwrap_or("");
    body.split("\"kind\": \"")
        .skip(1)
        .map(|chunk| {
            let kind = chunk.split('"').next().unwrap_or("").to_string();
            let level = chunk
                .split("\"level\": ")
                .nth(1)
                .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let text = chunk
                .split("\"text\": \"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();
            (kind, level, text)
        })
        .collect()
}

fn furniture(json: &str) -> Vec<String> {
    let f = json.split("\"furniture\"").nth(1).unwrap_or("");
    f.split('"')
        .skip(1)
        .step_by(2)
        .take_while(|s| !s.contains("footnote_markers"))
        .filter(|s| !s.trim().is_empty() && !s.contains(':'))
        .map(str::to_string)
        .collect()
}

/// Footnote markers: digits right after a word, which the truth leaves out.
fn drop_markers(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let digit = c.is_ascii_digit();
        let after_word = i > 0 && !chars[i - 1].is_whitespace() && !chars[i - 1].is_ascii_digit();
        let before_end = chars
            .get(i + 1)
            .is_none_or(|n| n.is_whitespace() || n.is_ascii_digit());
        if digit && after_word && before_end {
            continue;
        }
        out.push(c);
    }
    out
}

#[test]
fn books_reflow_like_the_source() {
    register_test_fonts();
    let dir = corpus("books/pdf");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pdf"))
        .collect();
    names.sort();
    let mut failures = Vec::new();
    for name in &names {
        let path = format!("{dir}/{name}");
        let json = std::fs::read_to_string(path.replace(".pdf", ".json")).unwrap();
        let truth = truth_blocks(&json);
        let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
        let mut got: Vec<(Kind, bool, String)> = Vec::new();
        for range in doc.reflow_chapters() {
            let r = doc.reflow(range).unwrap();
            for b in r.document.blocks {
                let note = b.style.font_scale.is_some_and(|s| s < 1.0);
                got.push((b.kind, note, b.text));
            }
        }
        let body_truth: Vec<&(String, u8, String)> = truth
            .iter()
            .filter(|(k, ..)| k == "heading" || k == "paragraph")
            .collect();
        let body_got: Vec<&(Kind, bool, String)> = got
            .iter()
            .filter(|(k, note, _)| !note && *k != Kind::Image)
            .collect();
        let t_text: String = body_truth.iter().map(|(.., t)| t.as_str()).collect();
        let g_text: String = body_got.iter().map(|(.., t)| drop_markers(t)).collect();
        let e = cer(&t_text, &g_text);
        // Block starts: the first letters of each block.
        let start = |s: &str| normal(s).into_iter().take(12).collect::<String>();
        let t_starts: Vec<String> = body_truth.iter().map(|(.., t)| start(t)).collect();
        let g_starts: Vec<String> = body_got
            .iter()
            .map(|(.., t)| start(&drop_markers(t)))
            .collect();
        let precision = g_starts.iter().filter(|s| t_starts.contains(s)).count() as f64
            / g_starts.len().max(1) as f64;
        let recall = t_starts.iter().filter(|s| g_starts.contains(s)).count() as f64
            / t_starts.len().max(1) as f64;
        let headings_ok =
            body_truth
                .iter()
                .filter(|(k, ..)| k == "heading")
                .all(|(_, level, t)| {
                    body_got
                        .iter()
                        .any(|(k, _, g)| *k == Kind::Heading(*level) && normal(g) == normal(t))
                });
        let leaks = got
            .iter()
            .filter(|(.., g)| {
                furniture(&json)
                    .iter()
                    .any(|f| normal(g).ends_with(&normal(f)[..]) && !normal(f).is_empty())
                    || (!g.trim().is_empty()
                        && g.trim()
                            .chars()
                            .all(|c| c.is_ascii_digit() || ('০'..='৯').contains(&c)))
            })
            .count();
        let notes = truth.iter().filter(|(k, ..)| k == "footnote").count();
        let notes_found = got.iter().filter(|(_, note, _)| *note).count();
        println!(
            "{name:34} cer {:5.2}%  starts P {:5.1}% R {:5.1}%  headings {}  leaks {leaks}  notes {notes_found}/{notes}",
            e * 100.0,
            precision * 100.0,
            recall * 100.0,
            if headings_ok { "ok" } else { "WRONG" }
        );
        // Mukta's two known glyph misses (see books.rs) cost a few letters.
        let cer_limit = if name.contains("mukta") { 0.01 } else { 0.005 };
        if e > cer_limit
            || precision < 0.95
            || recall < 0.95
            || !headings_ok
            || leaks > 0
            || notes_found != notes
        {
            failures.push(name.clone());
        }
    }
    assert!(failures.is_empty(), "reflow off target: {failures:?}");
}

#[test]
fn reflow_positions_map_back_to_pages() {
    register_test_fonts();
    let path = corpus("books/pdf/bn.chrome.two.pdf");
    let doc = PdfDocument::open(&Source::open(&path).unwrap()).unwrap();
    for range in doc.reflow_chapters() {
        let r = doc.reflow(range).unwrap();
        for (b, block) in r.document.blocks.iter().enumerate() {
            if block.kind == Kind::Image {
                continue;
            }
            // Every word of the reflow maps to the same word on its page,
            // and back.
            let mut offset = 0;
            for word in block.text.split(' ') {
                let (page, at) = r.to_page(b, offset).unwrap();
                let text = doc.page_text(page).unwrap().text;
                let first: String = word.chars().take(1).collect();
                assert!(
                    text[at..].starts_with(&first),
                    "block {b} word {word:?} maps to {:?}",
                    &text[at..(at + 12).min(text.len())]
                );
                assert_eq!(
                    r.from_page(page, at),
                    Some((b, offset)),
                    "back from page {page} offset {at}"
                );
                offset += word.len() + 1;
            }
        }
    }
}
