//! `akkhor`: open a book, render a page, and report time and memory.
//!
//! ```text
//! akkhor <file> [--page N] [--scale S] [--tile X,Y,W,H] [--size WxH]
//!               [--theme paper|sepia|dusk|night|original] [--out page.png] [--search TEXT]
//!               [--text]
//! ```
//!
//! `--text` prints the page's text (for PDFs: the repaired text layer, next
//! to how long extracting it took). `--search` lists hits with snippets.
//!
//! For PDFs `--page` is the page index; for EPUBs it is the chapter index and
//! the first page of that chapter is rendered.

use std::process::ExitCode;
use std::time::Instant;

use akkhor::{Book, Theme, Tile};

struct Args {
    file: String,
    page: usize,
    scale: f32,
    tile: Option<Tile>,
    size: (u32, u32),
    theme: Theme,
    out: Option<String>,
    search: Option<String>,
    text: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut args = Args {
        file: String::new(),
        page: 0,
        scale: 2.0,
        tile: None,
        size: (1080, 1920),
        theme: Theme::PAPER,
        out: None,
        search: None,
        text: false,
    };
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--page" => args.page = val()?.parse().map_err(|_| "bad --page")?,
            "--scale" => args.scale = val()?.parse().map_err(|_| "bad --scale")?,
            "--out" => args.out = Some(val()?),
            "--search" => args.search = Some(val()?),
            "--text" => args.text = true,
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').ok_or("--size is WxH")?;
                args.size = (
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                );
            }
            "--tile" => {
                let v: Vec<u32> = val()?
                    .split(',')
                    .map(|n| n.parse().map_err(|_| "bad --tile"))
                    .collect::<Result<_, _>>()?;
                if v.len() != 4 {
                    return Err("--tile is X,Y,W,H".into());
                }
                args.tile = Some(Tile::new(v[0], v[1], v[2], v[3]));
            }
            "--theme" => {
                args.theme = match val()?.as_str() {
                    "paper" => Theme::PAPER,
                    "sepia" => Theme::SEPIA,
                    "dusk" => Theme::DUSK,
                    "night" => Theme::NIGHT,
                    "original" => Theme::ORIGINAL,
                    t => return Err(format!("unknown theme {t}")),
                }
            }
            "-h" | "--help" => return Err(String::new()),
            f if args.file.is_empty() && !f.starts_with("--") => args.file = f.to_string(),
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    if args.file.is_empty() {
        return Err("no file given".into());
    }
    Ok(args)
}

/// Resident memory split into heap (anonymous) and mapped-file pages, in KB.
/// Mapped-file pages are the book itself, shared with the OS page cache and
/// dropped first under memory pressure; heap is what the engine allocated.
fn memory_split_kb() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| -> Option<u64> {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
    };
    Some((field("RssAnon:")?, field("RssFile:")?))
}

/// Peak and current resident memory in KB, from /proc (Linux and Android).
fn memory_kb() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
    };
    Some((field("VmHWM:")?, field("VmRSS:")?))
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn write_png(path: &str, w: u32, h: u32, premul: Vec<u8>) -> Result<(), String> {
    let mut px = premul;
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&px).map_err(|e| e.to_string())
}

fn run(args: Args) -> Result<(), String> {
    let file_mb = std::fs::metadata(&args.file)
        .map(|m| m.len() as f64 / 1e6)
        .unwrap_or(0.0);
    let t = Instant::now();
    let book = akkhor::open(&args.file).map_err(|e| e.to_string())?;
    let open_ms = ms(t);
    println!("file      {} ({file_mb:.1} MB)", args.file);
    println!("format    {:?}", book.format());
    println!("open      {open_ms:.2} ms");
    if let Some((heap, file)) = memory_split_kb() {
        println!(
            "memory    after open: heap {:.1} MB, mapped file {:.1} MB",
            heap as f64 / 1024.0,
            file as f64 / 1024.0
        );
    }

    if let Some(q) = &args.search {
        return search(&book, q);
    }
    if args.text {
        return print_text(&book, args.page);
    }

    let (w, h, pixels) = match &book {
        Book::Pdf(pdf) => {
            println!("pages     {}", pdf.page_count());
            let t = Instant::now();
            let (w, h, px) = match args.tile {
                Some(tile) => {
                    let mut buf = vec![0u8; tile.rgba_len()];
                    pdf.render_tile(args.page, args.scale, tile, &args.theme, &mut buf)
                        .map_err(|e| e.to_string())?;
                    (tile.width, tile.height, buf)
                }
                None => {
                    let p = pdf
                        .render_page(args.page, args.scale, &args.theme)
                        .map_err(|e| e.to_string())?;
                    (p.width, p.height, p.pixels)
                }
            };
            println!(
                "render    {:.2} ms  (page {}, {w}x{h} px)",
                ms(t),
                args.page
            );
            (w, h, px)
        }
        Book::Epub(epub) => {
            println!(
                "title     {}",
                epub.metadata.title.as_deref().unwrap_or("(untitled)")
            );
            println!(
                "chapters  {}  toc entries {}",
                epub.chapter_count(),
                epub.toc.len()
            );
            let t = Instant::now();
            let doc = epub.chapter(args.page).map_err(|e| e.to_string())?;
            println!(
                "chapter   {:.2} ms  ({} blocks, {} chars)",
                ms(t),
                doc.blocks.len(),
                doc.char_count()
            );
            let images = epub.chapter_images(args.page);
            render_flow(&doc, Some(&images), &args)?
        }
        Book::Text(text) => {
            println!("encoding  {:?}", text.encoding);
            render_flow(&text.document, None, &args)?
        }
    };

    if let Some((peak, now)) = memory_kb() {
        println!(
            "memory    total resident: peak {:.1} MB, now {:.1} MB",
            peak as f64 / 1024.0,
            now as f64 / 1024.0
        );
    }
    if let Some((heap, file)) = memory_split_kb() {
        println!(
            "memory    after render: heap {:.1} MB, mapped file {:.1} MB",
            heap as f64 / 1024.0,
            file as f64 / 1024.0
        );
    }
    if let Some(out) = &args.out {
        write_png(out, w, h, pixels)?;
        println!("wrote     {out}");
    }
    Ok(())
}

fn render_flow(
    doc: &akkhor::flow::Document,
    images: Option<&dyn akkhor::flow::ImageProvider>,
    args: &Args,
) -> Result<(u32, u32, Vec<u8>), String> {
    use akkhor::flow::{FontSet, GlyphCache, LayoutOptions, Paginator, render_page};
    let fonts = FontSet::bundled();
    let (w, h) = args.size;
    let opts = LayoutOptions {
        width: w as f32,
        height: h as f32,
        ..Default::default()
    };
    let t = Instant::now();
    let mut paginator = Paginator::new(doc, &opts, &fonts, images);
    let first = paginator.next().unwrap_or_default();
    println!(
        "page 1    {:.2} ms  (laid out {} of {} blocks, {} of {} fonts loaded)",
        ms(t),
        paginator.blocks_done(),
        doc.blocks.len(),
        fonts.loaded_count(),
        fonts.len()
    );
    let rest = paginator.count();
    println!("all pages {:.2} ms  ({} pages)", ms(t), rest + 1);
    let mut cache = GlyphCache::default();
    let mut px = vec![0u8; w as usize * h as usize * 4];
    let t = Instant::now();
    render_page(
        &first,
        &fonts,
        images,
        &mut cache,
        &args.theme,
        w,
        h,
        &mut px,
    )
    .map_err(|e| e.to_string())?;
    println!("render    {:.2} ms  ({w}x{h} px)", ms(t));
    Ok((w, h, px))
}

/// Prints every hit of `q` with its chapter and a snippet.
fn search(book: &Book, q: &str) -> Result<(), String> {
    use akkhor::flow::{find_all, snippet};
    let t = Instant::now();
    let mut shown = 0;
    let mut print =
        |chapter: Option<usize>, doc: &akkhor::flow::Document, m: &akkhor::flow::Match| {
            if shown < 20 {
                let (text, r) = snippet(doc, m, 25);
                let place = chapter.map_or(String::new(), |c| format!("ch {c:>3} "));
                println!(
                    "  {place}[{}:{}] …{}[{}]{}…",
                    m.start.block,
                    m.start.offset,
                    &text[..r.start],
                    &text[r.clone()],
                    &text[r.end..]
                );
            }
            shown += 1;
        };
    match book {
        Book::Epub(epub) => {
            for c in 0..epub.chapter_count() {
                let doc = epub.chapter(c).map_err(|e| e.to_string())?;
                for m in find_all(&doc, q) {
                    print(Some(c), &doc, &m);
                }
            }
        }
        Book::Text(text) => {
            for m in find_all(&text.document, q) {
                print(None, &text.document, &m);
            }
        }
        Book::Pdf(pdf) => {
            for m in pdf.search(q, 0) {
                if shown < 20 {
                    let text = pdf.page_text_cached(m.page).map_err(|e| e.to_string())?;
                    let (a, b) = (m.range.start, m.range.end);
                    let before = tail(&text.text[..a], 25);
                    let after = head(&text.text[b..], 25);
                    println!(
                        "  p {:>4} …{}[{}]{}…",
                        m.page + 1,
                        before.replace('\n', " "),
                        &text.text[a..b],
                        after.replace('\n', " ")
                    );
                }
                shown += 1;
            }
        }
    }
    println!("search    {shown} hits for {q:?} in {:.2} ms", ms(t));
    Ok(())
}

/// The last `n` characters of `s`.
fn tail(s: &str, n: usize) -> &str {
    let at = s
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    &s[at..]
}

/// The first `n` characters of `s`.
fn head(s: &str, n: usize) -> &str {
    let at = s.char_indices().nth(n).map_or(s.len(), |(i, _)| i);
    &s[..at]
}

/// Prints a page's text: a PDF page's repaired text layer, or an EPUB
/// chapter's / text file's paragraphs.
fn print_text(book: &Book, page: usize) -> Result<(), String> {
    match book {
        Book::Pdf(pdf) => {
            let t = Instant::now();
            let text = pdf.page_text(page).map_err(|e| e.to_string())?;
            let first = ms(t);
            let t = Instant::now();
            pdf.page_text(page).map_err(|e| e.to_string())?;
            println!(
                "text      page {} in {first:.2} ms (again: {:.2} ms)\n",
                page + 1,
                ms(t)
            );
            println!("{}", text.text);
        }
        Book::Epub(epub) => {
            let doc = epub.chapter(page).map_err(|e| e.to_string())?;
            for b in &doc.blocks {
                println!("{}\n", b.text);
            }
        }
        Book::Text(text) => {
            for b in &text.document.blocks {
                println!("{}\n", b.text);
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}");
            }
            eprintln!(
                "usage: akkhor <file> [--page N] [--scale S] [--tile X,Y,W,H] [--size WxH] [--theme paper|sepia|dusk|night|original] [--out page.png] [--search TEXT] [--text]"
            );
            ExitCode::FAILURE
        }
    }
}
