# Akkhor engine plan

> A tiny, fast reading engine that gets Indic scripts right: rendering, text, search and reflow.

This is the full plan for the engine. The reader app is a separate project
that starts once the engine is ready. Every phase below is planned here
before any code for it is written, and this file is updated as phases finish.

Last updated: 2026-11-25

---

## 1. Why this exists (the gaps we fill)

| # | Gap in current engines | What Akkhor does |
|---|---|---|
| 1 | Correct Indic rendering usually needs a heavy browser engine (WebView). Light native engines often break conjuncts. | Full OpenType shaping (HarfBuzz rules) in a small native engine. |
| 2 | Text in Indic PDFs is broken: copy, search and read-aloud give garbage. Legacy encodings (Bijoy/SutonnyMJ, Kruti Dev, Chanakya) make it worse. | Rebuild real Unicode text from PDF glyphs, plus legacy-encoding converters. |
| 3 | Line breaks, selection and highlights split conjuncts (they work on grapheme clusters, but an Indic syllable can be longer). | Everything works on orthographic syllables (aksharas). |
| 4 | Search misses matches typed differently (nukta, ZWJ/ZWNJ, ৎ vs ত্‍). | Indic-aware search normalisation. |
| 5 | No PDF reflow for Indic text. | Reflow built on the rebuilt text layer (gap 2). |
| 6 | Shipping every Indic font makes apps heavy. | A font is memory-mapped only when a book uses its script. |
| 7 | Multi-format engines are heavy or AGPL (MuPDF). | Pure Rust, MIT/Apache-2.0, every format a build switch. |

**Scripts.** MVP: Bangla, Devanagari (Hindi, Marathi, Nepali) and English.
Next: Gurmukhi, Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam (syllable
rules already cover them), then Sinhala, then right-to-left (Arabic, Urdu) and CJK.

## 2. Design rules

1. **Never read a whole file.** Books are memory-mapped; parsers touch only the bytes they need.
2. **Nothing runs for a format until a file of that format is opened.** Formats are Cargo features; a format that is off is not compiled at all.
3. **Every cache has a byte budget** (`LruCache`), so memory has a ceiling whatever the book size.
4. **One of each heavy part:** one rasterizer (vello_cpu) for PDF and text, one font parser (read-fonts) for shaping and outlines.
5. **Pure Rust, no C libraries.** Easy cross-compiling, memory safety on untrusted files.
6. **Positions survive re-layout:** a reading position is `(block, byte offset)`, never a page number.
7. **Never crash the host app:** every FFI entry point catches panics and returns an error.
8. **Every parser is fuzzed** before a release (books are untrusted input).

## 3. Architecture

```
akkhor (facade: open(), Book, format switches)
├── akkhor-core     Source (mmap), Format sniffing, LruCache, Theme, Tile, Error
├── akkhor-pdf      [pdf]   mmap'd PDF, tiled rendering into caller's buffer
│                           (hayro for PDF syntax, fonts and image codecs)
├── akkhor-flow     [flow]  the shared reflow pipeline:
│     markup  → tolerant HTML/XHTML tokenizer
│     model   → Document of blocks + style spans
│     script  → script detection, Indic syllable segmentation (9 scripts)
│     fonts   → per-script fonts, loaded lazily
│     shape   → HarfRust shaping per script run
│     layout  → syllable-safe line breaking, justification, pagination
│     paint   → glyph outline cache + vello_cpu rasterizing
├── akkhor-epub     [epub]  own zip reader (central directory only), OPF, nav/NCX
├── akkhor-cli      benchmark and render tool (not shipped)
└── akkhor-android  JNI bridge (libakkhor.so, class dev.akkhor.Native)
```

Fixed-layout formats (PDF, CBZ) go page → tiles → pixels.
Reflowable formats (EPUB, MOBI/AZW3, FB2, TXT) go format → `Document` → layout → pixels.

### Format switches

| Feature | Adds | Default |
|---|---|---|
| `pdf` | PDF | on |
| `pdf-cjk` | CJK character maps for Chinese/Japanese/Korean PDFs | off |
| `epub` | EPUB 2/3 (pulls in `flow`) | on |
| `txt` | plain text (pulls in `flow`) | on |
| `bundled-fonts` | Noto Serif Latin/Bengali/Devanagari compiled in (tests, CLI) | off |
| later: `mobi`, `fb2`, `cbz`, `legacy-bangla`, `legacy-hindi` | | |

## 4. Phases

Status: ✅ done · 🔄 in progress · ⏳ next · ⬜ planned

### Phase 0: Spike ✅ (2026-11-25)

Goal: prove the approach on real files before building on it.

- ✅ Workspace, format switches, error model
- ✅ Memory-mapped `Source`, format sniffing, byte-budget LRU cache, themes
- ✅ PDF: open from mmap, page sizes, tile rendering straight into a caller buffer
- ✅ Own zip reader (zip64 aware), EPUB OPF/spine/nav/NCX, chapter-on-demand
- ✅ HTML tokenizer and document model (headings, paragraphs, quotes, lists, bold/italic spans, `<br>`)
- ✅ Indic syllable segmentation for 9 scripts, script runs
- ✅ Lazy per-script fonts, HarfRust shaping, syllable-safe line breaking, justification (spaces only), pagination
- ✅ Glyph outline cache and page painting
- ✅ CLI benchmark tool; JNI bridge (host build verified, Android build in CI)

Results (Linux x86-64, release build):

| Test | Result |
|---|---|
| 308 MB / 2,000-page PDF, cold cache: open | 89 ms, 6.6 MB heap |
| same, warm: open | 5 to 13 ms |
| render one page at 1224×1584 | 7 to 14 ms |
| render a 512×512 zoomed tile | 3 ms |
| total resident memory, peak (cold) | 30 MB |
| Bangla EPUB chapter: open + parse + layout + render | under 8 ms total, ~15 MB peak |
| fonts loaded for a Bangla-only chapter | 1 of 3 |
| same 308 MB PDF on a GitHub CI runner (slow disk), cold open | 331 ms, 6.6 MB heap, 30 MB peak |
| `libakkhor.so`, all formats (CI, stripped) | arm64-v8a 5.5 MB, armeabi-v7a 4.1 MB, x86_64 6.7 MB |

Bangla (ক্ষ, ন্ত্র, র্ম, ব্যাঁ, ৎ, ZWNJ) and Devanagari (क्ष, त्र, ज्ञ, श्र, द्ध, ष्ट्र) render correctly.
Findings to act on:
- Cold PDF open is dominated by walking the whole page tree, which touches every page object on disk (fix in Phase 3).
- The Android library is over the 3 MB target. Main suspects: the 14 standard PDF fonts embedded by default, colour management and image codecs compiled in for every build. Size work is added to Phase 3.
- Justification needs hyphenation to avoid loose Latin lines (Phase 1).

### Phase 1: Indic text core 🔄

Make reflowable text complete and correct. Detailed design: [phases/phase-1.md](phases/phase-1.md).

1. Selection and hit-testing API: point → (block, offset), snapping to syllables; range → highlight rectangles.
2. Indic-aware search: normalise nukta forms, ZWJ/ZWNJ, khanda ta (ৎ / ত্‍), Bangla য়/ড়/ঢ় composed vs decomposed, Devanagari precomposed nukta letters; match across line breaks.
3. Line-breaking rules: no break before danda (। ॥) or closing punctuation, no break after opening punctuation; Latin hyphenation (patterns), optional Bangla/Hindi break hints.
4. Font fallback by coverage (a character the script font lacks falls to the next font), bold/italic font slots with synthetic bold/italic as fallback.
5. CSS subset: `text-align`, `text-indent`, `margin`, `font-size`, `font-weight`, `font-style`, `display:none`, `page-break-*`.
6. Images in EPUB (decode lazily, scale to page, byte-budgeted cache).
7. Incremental pagination: lay out the current chapter first, the rest in the background; cancel on font change.
8. TXT: encoding detection (UTF-8/16, Windows-1252) and Bijoy-encoded Bangla text detection.

Done when: a 1,000-page Bangla novel EPUB opens to its first page in under 150 ms on a mid-range phone, selection never splits a conjunct, and search finds every variant spelling in a test corpus.

### Phase 2: PDF Indic text layer ⬜ (the headline novelty)

1. Text extraction with positions from hayro's interpreter (glyph, font, transform).
2. Use `ToUnicode` maps when present and trustworthy.
3. Glyph-to-Unicode recovery when they are missing or wrong: reverse the font's cmap and GSUB ligature/conjunct rules to map shaped glyphs back to character sequences; reorder pre-base vowel signs back to logical order.
4. Legacy encodings: detect Bijoy/SutonnyMJ (Bangla) and Kruti Dev / Chanakya (Hindi) fonts by name and glyph usage; convert with mapping tables to Unicode.
5. Search, selection and copy on PDFs using this text layer.
6. Accuracy corpus: real Bangla and Hindi PDFs (Unicode, Bijoy, Kruti Dev), scored by character error rate.

Done when: copy/search works on the corpus with under 2% character errors.

### Phase 3: PDF speed and memory ⬜

1. Lazy page tree: find page N without visiting every page (target: cold open of the 2,000-page file under 30 ms).
2. Keep the render cache (fonts, images) alive per document instead of per tile.
3. Tile cache with a global byte budget; render cancellation when the user scrolls past.
4. Progressive rendering: low-resolution preview first, then sharp.
5. Smart night mode: draw text and lines in theme colours while images keep their real colours (our own drawing device, not whole-page inversion).
6. Thumbnails and table of contents (outlines).
7. Library size: measure what each dependency adds (`cargo bloat`), load the standard PDF fonts from disk instead of embedding them, put rarely used codecs (JPEG 2000, JBIG2, CCITT) behind switches, try `opt-level = "s"` for cold code. Target: under 3 MB for arm64-v8a with all formats.

### Phase 4: Indic PDF reflow ⬜

Turn a PDF page's text layer (Phase 2) into a `Document` (reading order, paragraphs, headings by size) and lay it out with the flow pipeline. Images kept as blocks.

### Phase 5: More formats ⬜

MOBI/AZW3 (PalmDB, PalmDOC and HUFF/CDIC decompression, KF8 skeleton/fragments → HTML → `Document`), FB2, CBZ. DRM-protected files are detected and reported, never opened.

### Phase 6: More scripts ⬜

Gurmukhi, Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam fonts and test corpora; Sinhala syllable rules; then bidi and right-to-left (Arabic, Urdu), then CJK line breaking.

### Phase 7: Hardening and release ⬜

`cargo-fuzz` targets for every parser, a real-world file corpus in CI, benchmarks against MuPDF as the bar to beat, Android AAR packaging, crates.io release, API docs.

After Phase 7 (or earlier, once Phases 1 to 3 land): the reader app, in its own repository.

## 5. Targets (to measure, not promises)

| Metric | Target | Spike |
|---|---|---|
| Engine `.so` size per CPU architecture, all formats | under 3 MB | 5.5 MB arm64 (work planned in Phase 3) |
| RAM while reading a 1 GB PDF | under 60 MB | 30 MB peak (308 MB file) |
| First page, any file size | under 300 ms | 89 ms cold (laptop), 331 ms cold (CI disk), 5 to 13 ms warm |
| Reopen | under 100 ms | yes |

## 6. Testing and CI

- Unit tests per crate (syllables in 9 scripts, shaping, layout invariants, zip, EPUB, sniffing).
- Every format switch built on its own, so switches never break.
- Android libraries built for arm64-v8a, armeabi-v7a and x86_64; sizes reported on every run.
- Benchmark job generates a 2,000-page PDF and reports open, render and memory numbers.
- Later: fuzzing, real-file corpus, rendering comparisons.

## 7. Decisions

| Decision | Choice | Why |
|---|---|---|
| Language | Rust | speed, memory safety on untrusted files, easy Android cross-compile |
| PDF interpreter | hayro (pure Rust, MIT/Apache) for syntax, fonts and image codecs; our own loading, tiling, caching and (later) drawing device | writing a PDF interpreter from scratch would take many months |
| Shaping | HarfRust 0.15 | HarfBuzz's Indic shapers; shares read-fonts with hayro, so one font parser |
| Rasterizer | vello_cpu | already used by hayro; one rasterizer for everything |
| Zip, markup, layout | written for Akkhor | small, and they are where the novelty lives |
| Android bridge | plain JNI (`jni-sys`), render into `Bitmap` memory | no copies, no extra runtime |
| License | MIT OR Apache-2.0 | usable by any app, unlike AGPL engines |
| Fonts | Noto Serif (SIL OFL) | free, good Indic coverage |

## 8. Risks

| Risk | Mitigation |
|---|---|
| hayro is young and labelled experimental | it has a 1,000+ file regression suite; we keep our own PDF corpus and can patch or replace parts |
| Glyph-to-Unicode recovery is hard for some fonts | corpus-driven work, legacy converters, fall back to visual-only |
| Engine size grows with features | size reported in CI on every change; heavy parts stay behind switches |
