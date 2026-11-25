# Akkhor (অক্ষর · अक्षर)

**A tiny, fast reading engine that gets Indic scripts right: rendering, text, search and reflow.**

Akkhor opens PDF, EPUB and text files, lays out and draws pages, and keeps
every conjunct whole: ক্ষ, ন্ত্র, র্ম, क्ष, त्र, श्र, द्ध. It is written in pure
Rust, memory-maps books instead of reading them, and lets you switch each
format on or off at build time.

> Status: early. The spike works (see [the plan](docs/PLAN.md) for results and the roadmap). The API will change.

## Why

Current engines either render Indic text correctly by carrying a whole browser
engine, or stay light and break conjuncts. PDFs in Bangla and Hindi give
garbage when you copy or search, especially legacy Bijoy and Kruti Dev files.
Akkhor aims to be light **and** correct. The full gap analysis is in
[docs/PLAN.md](docs/PLAN.md#1-why-this-exists-the-gaps-we-fill).

## What works today

- **PDF:** opens from a memory map (a 308 MB, 2,000-page file opens in 5 to 89 ms with ~7 MB of heap), renders any tile of any page at any zoom straight into your pixel buffer.
- **EPUB 2/3:** own zip reader that only reads the archive index, metadata, reading order, table of contents, chapters parsed on demand.
- **Reflowable text:** OpenType shaping (HarfRust), orthographic-syllable segmentation for Bangla, Devanagari, Gurmukhi, Gujarati, Odia, Tamil, Telugu, Kannada and Malayalam, line breaking that never splits a conjunct, justification that only widens spaces, pagination, themes (paper, sepia, dusk, night).
- **Fonts on demand:** a script's font is only loaded when a book uses that script.
- **Android:** a JNI bridge that draws straight into a `Bitmap`.

## Format switches

```toml
[dependencies]
akkhor = { version = "0.1", default-features = false, features = ["epub"] }  # EPUB only
```

| Feature | Adds |
|---|---|
| `pdf` (default) | PDF |
| `epub` (default) | EPUB 2 and 3 |
| `txt` (default) | plain text |
| `pdf-cjk` | CJK character maps for Chinese, Japanese and Korean PDFs |
| `bundled-fonts` | Noto Serif Latin, Bengali and Devanagari compiled in |

A format that is off is not compiled. A format that is on uses no memory until you open a file of that format.

## Try it

```sh
cargo run --release -p akkhor-cli -- tests/fixtures/indic-sample.epub --page 0 --out page.png
cargo run --release -p akkhor-cli -- tests/fixtures/indic.pdf --scale 2 --theme sepia --out pdf.png

# a big PDF for benchmarking (2,000 pages, ~300 MB)
python3 scripts/make_fixtures.py --big 2000 --image-kb 150
cargo run --release -p akkhor-cli -- tests/fixtures/big.pdf --page 1999
```

## Use it from Rust

```rust
use akkhor::{Book, Theme};

let book = akkhor::open("book.pdf")?;
if let Book::Pdf(pdf) = &book {
    let page = pdf.render_page(0, 2.0, &Theme::PAPER)?; // RGBA8, premultiplied
    println!("{} x {}", page.width, page.height);
}
```

## Layout

| Crate | Role |
|---|---|
| `akkhor` | the facade: `open()`, `Book`, format switches |
| `akkhor-core` | memory-mapped sources, format sniffing, byte-budget caches, themes |
| `akkhor-pdf` | PDF loading and tiled rendering |
| `akkhor-flow` | the reflow pipeline: markup, document model, Indic syllables, fonts, shaping, layout, painting |
| `akkhor-epub` | zip and EPUB |
| `akkhor-cli` | benchmark and render tool |
| `akkhor-android` | JNI bridge (`libakkhor.so`) |

## License

Code: MIT or Apache-2.0, at your option. Fonts in `fonts/`: SIL Open Font License 1.1 (Noto).

PDF interpretation uses [hayro](https://github.com/LaurenzV/hayro); shaping uses [HarfRust](https://github.com/harfbuzz/harfrust); rasterizing uses [vello_cpu](https://github.com/linebender/vello).
