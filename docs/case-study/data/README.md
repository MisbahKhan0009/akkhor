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
| `memory.csv` | resident memory added to open a file and render one page | Peak (`VmHWM`, reset through `/proc/self/clear_refs`) minus resident memory just before opening; median of 3 fresh processes. Poppler: peak of the `pdftoppm` child minus that of `pdftoppm -v`. |
| `size.csv` | library size | `strip --strip-unneeded` on a copy of each engine's main shared library, x86_64 Linux |
| `robustness.csv` | 200 damaged PDFs: opened, refused with an error, crashed, hung | Deterministic mutations (seed 20261125) of four PDFs: bytes flipped, file cut short, a block duplicated or zeroed. Each file opened, page 1 rendered at 0.5x and its text read, in a fresh process with a 30 s limit. "Opened" means page 1 was shown: a file opened with no pages counts as refused, for every engine (until 2026-11-25 such files counted as opened for MuPDF and PDFium, which made MuPDF's lead look larger). |
| `books.csv` | book-layout corpus (tests/corpus/books, 22 PDFs from Chrome, Word via LibreOffice, XeLaTeX, Typst): per engine and file, words intact and found in the page text; for Akkhor also reflow scores | `scripts/bench/score_books.py`. Words intact: the truth's words found as whole words (not run together or split). Reflow: CER against the truth blocks (footnotes and images apart), block-start precision and recall, headings at the right level, footnotes found, running headers or page numbers leaked |
| `books_summary.csv` | `books.csv` averaged per engine and producer | same |
| `real_world_book.csv` | a real 204-page Bangla book (copyrighted: measured locally, not committed), Bangla words intact per engine | `scripts/bench/score_local.py`; reference = Akkhor with the book's fonts (Kalpurush, Hind Siliguri, both OFL) registered, spot-checked on 3 page images; bag of words, so reading order does not count |
| `real_world_findings.csv` | defects that book exposed, before and after the fix, and the corpus file that now reproduces each | `crates/akkhor-pdf/tests/quirks.rs` |
| `phase4_fixes.csv` | each Phase 4 text-layer fix, before and after | named per row |
| `reflow_timing.csv` | page structure and reflow time alone (page text already built), per document | `crates/akkhor-pdf/examples/reflowtime.rs` |
| `history.csv` | one row per milestone and metric | each row names where it was measured and its source |

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
