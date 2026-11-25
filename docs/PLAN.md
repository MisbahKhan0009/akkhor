# Akkhor engine plan

> A tiny, fast reading engine that gets Indic scripts right: rendering, text, search and reflow.

This is the full plan for the engine. The reader app is a separate project
that starts once the engine is ready. Every phase below is planned here
before any code for it is written, and this file is updated as phases finish.

Last updated: 2026-11-26 (Phase 5 planned)

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

### Phase 1: Indic text core ✅ (2026-11-25, reviewed: [reviews/phase-0-1.md](reviews/phase-0-1.md))

Make reflowable text complete and correct. Detailed design and checklist: [phases/phase-1.md](phases/phase-1.md).

- ✅ 1.1 Positions, hit-testing, word selection, highlight rectangles, copy; all snap to syllables
- ✅ 1.2 Indic-aware search (nukta, split vowels, ৎ / ত্‍, ZWJ/ZWNJ, case, quotes); never matches half a conjunct; lazy whole-book EPUB search
- ✅ 1.3 No line starts with । ॥ or closing punctuation; English hyphenation (Liang, hyph-utf8 en-us)
- ✅ 1.4 Font fallback by coverage; bold/italic faces with synthetic fallback
- ✅ 1.5 CSS subset: alignment, indent, margins, font size/weight/style, display:none, page breaks; linked, embedded and inline styles
- ✅ 1.6 EPUB images: header-only sizing, PNG/JPEG decoding at display size, byte-budget cache
- ✅ 1.7 Incremental pagination (`Paginator`), `page_for(position)`
- ✅ 1.8 TXT: UTF-8/UTF-16/Windows-1252 decoding, Bijoy detection
- ✅ 1.9 Android bridge (hitTest, wordAt, selectionRects, textBetween, search) and CLI `--search`

Results (GitHub CI runner, release build):

| Test | Result |
|---|---|
| ~1,000-page Bangla text (1,072 pages): first page | 0.75 ms (6 of 6,000 paragraphs laid out) |
| same, all pages | 269 ms |
| search the same text (1,500 hits) | 10 ms |
| EPUB chapter with an image: layout + render | 2.4 ms + 13 ms |
| Bijoy-encoded sample | detected as `LegacyBijoy` |
| `libakkhor.so` arm64-v8a, all formats | 5.8 MB (was 5.5 MB; size work is in Phase 3) |

Known limits carried forward: adjacent-sibling and attribute CSS selectors are ignored; GIF and WebP images are sized but not drawn; Bijoy text is detected but not yet converted (done in Phase 2).

### Phase 2: PDF Indic text layer ✅ (the headline novelty, reviewed)

Detailed design: [phases/phase-2.md](phases/phase-2.md). Measured starting point: a Chrome-printed Bangla/Hindi PDF reads back as `ক্ষম , ম  , কম` for `ক্ষমা, মন্ত্রী, কর্ম`.

1. ✅ Text layer extraction (glyph boxes, `ToUnicode`, fonts).
2. ✅ Recover letters for every glyph. Revised: producers strip `GSUB` from embedded subsets, so each glyph is found by its outline in a complete reference font (bundled or registered by the app) and that font's shaping rules are run in reverse.
3. ✅ Visual-to-logical reordering (pre-base vowel signs, split vowels, reph, fused reph forms).
4. ✅ Lines, words and reading order, with every character mapped to its box. Result: 0.0% character errors on all 32 corpus PDFs (two producers, two designs per script, with and without `ToUnicode`); read glyph by glyph through `ToUnicode` alone, the same PDFs are 2 to 45% wrong. (Correction from the Phase 3 engine comparison: MuPDF, PDFium and Poppler also read the `/ActualText` spans Chrome and LibreOffice write and get under 1% on these PDFs; Akkhor reads them too since Phase 3. See `docs/case-study/`.)
5. ✅ Legacy encodings: Bijoy (table from the MIT `poriborton` crate, inverted) for PDFs and `.txt`. Kruti Dev moved later: no open table or corpus to verify one against.
6. ✅ Search, selection and copy on PDFs (CLI and Android).
7. ✅ Accuracy corpus: 36 PDFs from two producers, two designs per script, Bijoy, with and without `ToUnicode`; character error rate checked in CI.

Review: [reviews/phase-2.md](reviews/phase-2.md), 16 findings; the wrong-output ones fixed (Unicode "OMJ" fonts treated as Bijoy, learning from malformed candidates, Assamese spellings claiming Bangla glyphs). Carried forward: Chrome's synthetic bold (Type 3 fonts), fonts the engine does not know, no real-world PDFs in the corpus yet, UTF-16 positions for read-aloud.

### Phase 3: PDF speed, memory and size ✅

Detailed design: [phases/phase-3.md](phases/phase-3.md). Measured starting point: night and dusk themes make PDF text invisible (dark text drawn on the dark background); cold open of a 2,000-page PDF takes 333 ms because every page dictionary is read at open; each zoomed tile re-interprets the whole page; the arm64 library is 6.27 MB.

1. Themes on PDFs: remap page colours by lightness (text in the theme's colour, hue kept), images keep their colours.
2. Lazy page tree (vendored `hayro-syntax`, small documented patch): page count from `/Count`, page N found by descending the tree. Target: cold open under 30 ms.
3. Batch tile rendering with one shared render cache per page. Target: later tiles under half the first's cost.
4. Cancellation between tiles; a stated byte budget for every engine cache.
5. Thumbnails and table of contents (PDF outlines), in the CLI and on Android.
6. Library size: packed relocations, size-optimised cold code, JBIG2/JPEG 2000/CCITT as switches, standard fonts from a file. Target revised to 3.5 MB for all formats (the PDF text recovery needs the shaper), under 3 MB for PDF-only without rare codecs.
7. Case study material (`docs/case-study/`): reproducible comparison with MuPDF, PDFium and Poppler (Indic text accuracy, search, open time, render time, memory, size, robustness), charts, a long-form draft and a LinkedIn post draft. From now on every phase adds its numbers to `docs/case-study/data/history.csv`.
8. CI benchmarks and the review.

Results: night, dusk and sepia work on PDFs with photos kept in colour; cold open of the 2,000-page PDF 333 → 12 ms on CI; redraw of an Indic page 5.4 → 3.0 ms; later tiles of a page about 1.1 ms against 6.1 ms for the first; cancellation between tiles; `trimMemory`; outlines and thumbnails; arm64 library 6.42 → 5.81 MB (size target missed, revised). Comparison with MuPDF, PDFium and Poppler, figures and drafts in [`case-study/`](case-study/).

Review: [reviews/phase-3.md](reviews/phase-3.md), 13 findings. Fixed: night-theme cost growing with the number of images on a page, the hayro version the render cache's `unsafe` relies on now pinned, benchmark load check. Carried forward: library size, redraw speed against MuPDF, memory on Indic pages, damaged-file recovery.

### Phase 4: Indic PDF reflow ✅

Detailed design: [phases/phase-4.md](phases/phase-4.md). Measured starting point: no reflow; on a two-column XeLaTeX book page 8.7% of words run together (XeLaTeX draws no spaces and justifies tightly; MuPDF finds 17% of words there); running headers and page numbers are read as text lines; line-end hyphens are kept.

1. A book-like corpus with ground truth: Chrome, LibreOffice Writer (.docx, .odt), XeLaTeX, LuaLaTeX (and Typst); one and two columns, headers, page numbers, footnotes, hyphenation, headings, figures. A `tests/real/` folder for real PDFs used with permission.
2. Word gaps from the font's space width and per-line gap clusters. Target: 100% of words found on XeLaTeX and LuaLaTeX PDFs.
3. Page structure (`akkhor-pdf::structure`): columns, running headers and page numbers, paragraphs, headings, line-end hyphens, footnotes, bold and italic, images.
4. Reflow: chapters from the outline (or 16-page runs), a `Document` per chapter laid out by the EPUB pipeline, a two-way position map between reflow and page positions. Targets: reflow error under 0.5%, block boundaries over 95%, no headers or page numbers leaked.
5. Interfaces: CLI `--reflow`, the `akkhor` crate, Android (reflow chapters, position conversion, UTF-16 offsets for read-aloud).
6. Type 3 synthetic-bold headings recovered by outline matching.
7. Carried from Phase 3: library size (target under 5.5 MB), damaged-file recovery (rebuild the cross-reference table), memory on Indic pages (share parsed fonts), a redraw study.
8. Benchmarks, case study (reflow section and book corpus for every engine) and the review.

Moved out: the downloadable Bangla and Hindi font pack (app side, licence checks), Kruti Dev (no verifiable table).

Results: reflow on 22 book PDFs from four producers with 0.0% wrong characters, every paragraph and heading break, footnotes and no running headers or page numbers, under 1 ms per chapter; positions map both ways. Text layer: XeLaTeX word gaps, Typst ActualText spans, Chrome's clipped next-page glyphs and Type 3 synthetic bold (read by shape when there is no text). A real 204-page Bangla book (measured locally) found fake bold drawn twice and Ghostscript-rewritten fonts: 100% of its words whole with its free fonts registered, against about 26% for MuPDF, PDFium and Poppler. Standard PDF fonts can come from the app; arm64 library 5.73 MB.

Review: [reviews/phase-4.md](reviews/phase-4.md), 21 findings; the four wrong-output or crash ones fixed (stack overflow on hostile pages, doubled letters from fake bold and from Typst, Type 3 advances). Two of the Phase 3 comparison's claims were corrected (damaged files, memory). Carried: library size (0.23 MB over), tables and cards in reflow, Mukta's two glyphs, hyphen-aware page search, a downloadable font pack for the app.

### Phase 4b: tables, cards and open items ✅

Detailed design: [phases/phase-4b.md](phases/phase-4b.md). From the Phase 4 review: tables are read column by column, cards and boxes are not seen, bold lead-ins become headings, Mukta's two glyphs, page search misses hyphenated words, 2 damaged files, one-page furniture.

1. Table and card corpus (Chrome, Word via LibreOffice, XeLaTeX, Typst; Bangla and Hindi) with truth.
2. Rules and boxes collected from the page's vector shapes.
3. Tables (ruled, horizontal rules only, unruled) found before the column cut; reflow as text when up to 3 columns, as a picture with alt text when wider or irregular.
4. Cards read box by box.
5. Centred text and bold lead-in rules.
6. Mukta candrabindu and nukta conjuncts.
7. Hyphen-aware page search.
8. Damaged files, one-page furniture.
9. Scoring, case study and review.

Results: on 8 table and card PDFs from four producers every block reflows as the truth says: small tables row by row with cells in order, wide ones as a picture with alt text, cards whole; MuPDF, PDFium and Poppler keep 65%, 76% and 48% of table rows in order in their text. The real Bangla book's tables now read across (585 rows on 53 pages) and its reflow has 356 headings instead of 1,221. Page search finds hyphenated words; 145 of 200 damaged files show a page; the book corpus did not move.

Review: [reviews/phase-4b.md](reviews/phase-4b.md), 18 findings; the six wrong-output ones fixed (cell gaps, line spacing, a cut-off table row, numbering columns, labels taken for cards and headings, Typst reph). Carried: a permitted real PDF with tables, header labels in coloured boxes, headings followed by a table, one text-layer reorder seen on the real book.

### Phase 5: Ready for the app 🔄

Detailed design: [phases/phase-5.md](phases/phase-5.md). What the reader app needs from the engine, behind an interface that stops changing: book info and covers, locators that survive font and view changes, progress, whole-book search, ranges for highlights, sentences for read aloud, a PDF page-range writer, colour themes from the app, a documented and tested JNI interface (`docs/api/android.md`, `Native.kt`, `apiVersion`), fuzzing in CI, a missing-font hint, and numbers from a real phone.

### Phase 6: More formats ⬜

MOBI/AZW3 (PalmDB, PalmDOC and HUFF/CDIC decompression, KF8 skeleton/fragments → HTML → `Document`), FB2, CBZ. DRM-protected files are detected and reported, never opened.

### Phase 7: More scripts ⬜

Gurmukhi, Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam fonts and test corpora; Sinhala syllable rules; then bidi and right-to-left (Arabic, Urdu), then CJK line breaking.

### Phase 8: Hardening and release ⬜

Fuzzing kept up as formats arrive (started in Phase 5), a real-world file corpus in CI, benchmarks against MuPDF as the bar to beat, Android AAR packaging, crates.io release, API docs.

After Phase 5: the reader app, in its own repository (designed separately); Phases 6 to 8 continue alongside it.

## 5. Targets (to measure, not promises)

| Metric | Target | Spike |
|---|---|---|
| Engine `.so` size per CPU architecture, all formats | under 3 MB (revised: under 5.5 MB, see `docs/reviews/phase-3.md`) | 5.81 MB arm64 after Phase 3; PDF only 4.97 MB; EPUB and TXT 2.19 MB |
| RAM while reading a 1 GB PDF | under 60 MB | 30 MB peak (308 MB file) |
| First page, any file size | under 300 ms | PDF: 89 ms cold (laptop), 338 ms cold (CI disk), 5 to 13 ms warm. Long Bangla text: under 1 ms |
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
