# Case study data

Produced by `scripts/bench/compare_engines.py` (comparison) and kept by
hand in `history.csv` (milestones). Machine, versions and date of the last
run: `environment.json`.

| File | What | Method |
|---|---|---|
| `text_accuracy.csv` | per engine and corpus PDF: character error rate (`cer`), share of the source's words found by search (`words_found`), extraction time | CER = edit distance / length of the source text, after Unicode canonical decomposition (NFD), with whitespace and zero-width joiners removed. A word is found when it appears, NFD-normalized, in the extracted text with whitespace collapsed. |
| `samples.json` | the first line each engine extracts from five corpus PDFs | as extracted, no normalization |
| `speed.csv` | open, first render and second render of one page; best and median of 5 runs | Fresh process per engine; each run opens the file anew. Libraries timed inside the process around the call. Poppler is driven through `pdfinfo` and `pdftoppm`; the median start-up of `pdfinfo -v` is subtracted (`startup_subtracted_ms`) and the raw median is kept (`raw_median_ms`). |
| `cold_open.csv` | open time with the file not in the OS cache | `/proc/sys/vm/drop_caches` before each of 3 runs; median. Poppler left out (its own start-up is also cold). |
| `memory.csv` | resident memory added to open a file and render one page (every library renders four bytes a pixel, as Android bitmaps are; Poppler's `pdftoppm` uses three, which favours it) | Peak (`VmHWM`, reset through `/proc/self/clear_refs`) minus resident memory just before opening; median of 3 fresh processes. Poppler: peak of the `pdftoppm` child minus that of `pdftoppm -v`. |
| `size.csv` | library size | `strip --strip-unneeded` on a copy of each engine's main shared library, x86_64 Linux |
| `robustness.csv` | 200 damaged PDFs: opened, refused with an error, crashed, hung | Deterministic mutations (seed 20261125) of four PDFs: bytes flipped, file cut short, a block duplicated or zeroed. Each file opened, page 1 rendered at 0.5x and its text read, in a fresh process with a 30 s limit. "Opened" means page 1 was shown: a file opened with no pages counts as refused, for every engine (until 2026-11-25 such files counted as opened for MuPDF and PDFium, which made MuPDF's lead look larger). |
| `books.csv` | book-layout corpus (tests/corpus/books, 22 PDFs from Chrome, Word via LibreOffice, XeLaTeX, Typst): per engine and file, words intact and found in the page text; for Akkhor also reflow scores | `scripts/bench/score_books.py`. Words intact: the truth's words found as whole words (not run together or split). Reflow: CER against the truth blocks (footnotes and images apart), block-start precision and recall, headings at the right level, footnotes found, running headers or page numbers leaked |
| `books_summary.csv` | `books.csv` averaged per engine and producer | same |
| `real_world_book.csv` | a real 204-page Bangla book (copyrighted: measured locally, not committed), Bangla words intact per engine | `scripts/bench/score_local.py`; reference = Akkhor with the book's fonts (Kalpurush, Hind Siliguri, both OFL) registered, spot-checked on 3 page images; bag of words, so reading order does not count |
| `real_world_findings.csv` | defects that book exposed, before and after the fix, and the corpus file that now reproduces each | `crates/akkhor-pdf/tests/quirks.rs` |
| `phase4_fixes.csv` | each Phase 4 text-layer fix, before and after | named per row |
| `reflow_timing.csv` | page structure and reflow time alone (page text and shapes already built), per document; the two passes before it (page text; pictures, rules and boxes) timed apart. One row per phase | `crates/akkhor-pdf/examples/reflowtime.rs` |
| `tables.csv` | table and card corpus (tests/corpus/tables, 8 PDFs from Chrome, Word via LibreOffice, XeLaTeX, Typst): per engine and file, words intact, table rows whose cells come out one after another in order, cards whose texts come out together; for Akkhor the same on its reflow, plus text-mode rows that are one block and wide tables shown as a picture | `scripts/bench/score_tables.py`. The other engines are scored on their page text: they have no reflow, so this is the order a reader or a screen reader gets from them |
| `tables_summary.csv` | `tables.csv` averaged per engine and producer | `scripts/bench/score_tables.py --summary` |
| `real_world_tables.csv` | the real Bangla book (not committed): headings, table rows and cards in the reflow, Phase 4 against Phase 4b | `akkhor-bench reflow` with the book's fonts registered, both builds; spot-checked on page images |
| `phase4b_fixes.csv` | each Phase 4b fix, before and after | named per row |
| `history.csv` | one row per milestone and metric | each row names where it was measured and its source |
| `page_copies.csv` | Phase 5 page writer: per test PDF, the pages copied by Akkhor, then read in MuPDF, PDFium and Poppler | `scripts/bench/check_page_copies.py`: every page of files up to 4 pages (last first), else the last and first; same text (whitespace collapsed) and pixels at 72 dpi (a channel off by more than 8 counts); `qpdf --check` on the copy |
| `phase5_fixes.csv` | the four wrong-output, crash and hang bugs fixed in Phase 5, with how each was found | from `docs/reviews/phase-5.md` |
| `formats_scores.csv` | Phase 6: words found in order, words found, extra words and time, per Kindle or FB2 book and engine (Akkhor, MuPDF, KindleUnpack, Calibre), against the source EPUB's text | `scripts/bench/score_formats.py` |
| `phase6_vm.csv` | Phase 6: every new-format test book through `akkhor-bench phone` on the VM, cold and warm | `akkhor-bench phone` |
| `size_phase6.csv` | Phase 6: arm64 `libakkhor.so` for the app build with and without each new format | CI, "Sizes per feature set (arm64)" |
| `glyph_masks.csv` | Phase 6: reflowed screens drawn from outlines and from cached glyph masks: first, next and repeat draws, cache size, pixel difference | `crates/akkhor/examples/drawtime.rs` |
| `ref_tables.csv` | Phase 6: reference-font tables built, then read back from the cache folder | `crates/akkhor-pdf/examples/reftimes.rs` |
| `fuzz_phase6.csv` | Phase 6: fuzz rounds for the MOBI, FB2 and comic targets and what each found | `cargo fuzz run -s none` |
| `phone_phase6.csv` | Phase 6 on the Realme 9i: the Phase 5 books plus an AZW3, a MOBI 7, an FB2 and a comic, cold and warm passes (each a new process). The reference-font tables were not kept on disk in this run (see the Phase 6 review, finding 19) | `scripts/phone/on-phone.sh` |
| `phone_phase6_library.csv` | the same run's library screen: every book's card in one process, twice | `akkhor-bench library` |
| `phone_phase6_run2.csv` | the second Phase 6 phone run, through `watch.sh`, with the tables kept on disk; slower across the board than the first (screen likely off, see the Phase 6 review, finding 22) | `scripts/phone/watch.sh` |
| `phone_phase6_run3.csv`, `phone_phase6_run3_library.csv` | the third Phase 6 phone run (screen on, through `watch.sh`), after the font-key change: the cleanest Phase 6 phone numbers | `scripts/phone/watch.sh` |
| `phone.csv` | Phase 5 on a real phone (Realme 9i, Snapdragon 680, Android 13) and the cloud VM, same books and steps; one row per book and pass (each pass a new process, like the app starting) | `akkhor-bench phone FILE` (`scripts/phone/`): 1080 × 2200 screen, 42 px text; times in ms, memory in MB added at peak |
| `phone_summary.csv` | median of the warm passes per format and measure, phone against VM | from `phone.csv` |

## What to keep in mind

- **The corpus is ours.** The texts were written for Akkhor and printed to
  PDF by Chrome and LibreOffice in four fonts (Noto Serif Bengali and
  Devanagari, Hind Siliguri, Mukta), plus Bijoy text in a font named
  SutonnyMJ. Real-world PDFs from Word, InDesign or publishers are not in
  it yet.
- **"No text layer"** files are the same PDFs with every `ToUnicode` map
  and `/ActualText` span removed: only the glyphs are left. Old or broken
  producers write such files; a scan of a page is a different problem
  (it needs OCR) and is not measured.
- **"Akkhor (+ device fonts)"** registers the PDF's font as a reference,
  as the Android app does with the phone's fonts. Plain "Akkhor" only knows
  its bundled Noto fonts.
- **Not like for like:** library sizes cover different feature sets;
  Poppler numbers include command-line overhead even after subtraction;
  the machine is a 2-core cloud VM, so absolute times are slower than a
  desktop and faster than a budget phone.
