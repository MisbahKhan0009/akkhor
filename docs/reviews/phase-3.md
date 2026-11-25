# Review: Phase 3 (PDF speed, memory and size)

Date: 2026-11-25. Scope: themes (`theme.rs`), the lazy page tree in
`vendor/hayro-syntax`, the shared render cache, batch tiles and
cancellation, `trimmed()`, outlines and thumbnails, the codec switches and
size settings, the Android bridge additions, the CI benchmark and the
case-study scripts, as of commit `74a0e69` plus the fixes below. Method:
line-by-line reading against the code-review checklist (correctness,
performance, robustness, maintainability), every `unsafe` block checked
against the upstream source it relies on, hostile inputs built for the new
code paths, and the full engine comparison rerun on an idle machine.

Severity: 🔴 must fix (wrong output, crash) · 🟠 should fix (performance
cliffs, fragile behaviour, missed targets) · 🟡 improve or document.

## Evidence

| Check | Result |
|---|---|
| Renders before and after every Phase 3 change, all corpus and fixture PDFs | identical (`scripts/compare_renders.sh`: 0 differences) |
| Text accuracy, 36 corpus PDFs | unchanged: 0.0% with the PDF's fonts registered |
| Cold open, 2,000-page 308 MB PDF (CI) | 333 ms → **12 ms** (target under 30 ms) |
| Warm open, same file (comparison run) | 0.16 ms (MuPDF 0.38, PDFium 0.91) |
| Redraw of the Bangla and Hindi page at 2x | 5.4 → **3.0 ms**; MuPDF 1.1, PDFium 3.7 |
| 512×512 tiles of an Indic page at 4x | first 6.1 ms, later ones about 1.1 ms (target: under half) |
| Heap over a 200-page session | +7 KB a page; 2.1 → 1.1 MB after `trimmed()` |
| Page drawing 100,000 small colour images, night theme at 2x | **+450 ms over paper** before the fix, +40 ms after (finding 1) |
| 200 damaged PDFs | Akkhor opens 143, refuses 57, 0 crashes or hangs (MuPDF 180, PDFium 109, Poppler 117) |
| `libakkhor.so` arm64, all formats | 6.42 → **5.81 MB**; target 3.5 MB **not met** (finding 2) |
| arm64 per feature set (CI) | no scan codecs 5.37 MB · PDF only, no scans 4.97 MB · EPUB and TXT 2.19 MB |
| CI on the newest stable toolchain | **failed once** on a new clippy lint (finding 9) |

## Findings

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 1 | `theme::apply` | Each pixel was checked against every image box on its row. A page that draws many images (tiled backgrounds, mosaics, some scanned comics) made the night theme cost pixels × images: 100,000 small images added 450 ms at 2x, on top of the render. | 🟠 | Fixed: rows are swept top to bottom with the boxes that cover them and a difference array per row, so the cost is the pixels plus the boxes' heights (+40 ms on that page). New test checks 300 overlapping boxes pixel by pixel against the plain definition. |
| 2 | size | arm64 with all formats is 5.81 MB, not 3.5 MB. Packed relocations, size-optimised cold crates and the codec switch together saved 0.62 MB. What remains is mostly code the engine needs in every build: the Rust standard library, HarfRust, read-fonts and skrifa (shaping is needed both to lay out EPUBs and to recover PDF text), the PDF parser and interpreter, and the rasterizer. Even a PDF-only build without scan codecs is 4.97 MB. | 🟠 | Target missed and revised, not hidden: under 5.5 MB for all formats, under 4.5 MB PDF-only, both tracked in CI. Remaining steps moved to Phase 4: standard fonts from a file (0.27 MB), a `cargo bloat` pass on arm64, dropping unused HarfRust tables. Shipping per-ABI APKs keeps the download at one library. |
| 3 | `render_cache` | `unsafe impl Send` and the lifetime transmute are sound only because hayro 0.9.0's `RenderCache` hands its `Rc` clones (glyph and outline caches) to the context of one render, which is dropped before the call returns (checked in hayro `src/lib.rs`), and `Pdf` holds no `Rc` (it is `Sync`). A minor hayro release could change that silently, since the dependency was `0.9`. | 🟡 | Fixed: hayro pinned to `=0.9.0` with a comment saying what to re-read before raising it. |
| 4 | Android `cancelRendering` | A render takes its generation when it starts. A cancel that arrives before the render thread picks up the job is not seen, and the stale tiles are drawn. | 🟡 | Documented: the app drops queued jobs itself and uses `cancelRendering` for the one that is running. |
| 5 | `image_boxes` cache | Boxes per page are not capped (32 bytes an image). The hostile page above keeps 3.2 MB for each of the 16 cached pages. | 🟡 | Documented; bounded by the number of images the interpreter actually draws, which costs far more time first. |
| 6 | redraw | Akkhor still redraws a page about 3x slower than MuPDF (3.0 vs 1.1 ms on the Indic page): hayro interprets the content stream again on every draw, and `vello_cpu` 0.3 cannot record a page and replay it. | 🟠 | Open. Tiles make it matter less (only visible tiles are drawn); recording a display list is a Phase 4 study. |
| 7 | memory | An Indic page at 2x adds 11.4 MB against 7.7 to 9.1 MB for the others (glyph outlines kept per size, fonts parsed for text recovery and for rendering separately). | 🟠 | Open, Phase 4: share parsed fonts between recovery and rendering; measure per cache. |
| 8 | robustness | MuPDF opens 37 more of the 200 damaged files. Akkhor refuses them cleanly, but a reader would rather show what it can. | 🟠 | Open, Phase 4: rebuild a damaged cross-reference table by scanning for objects, as MuPDF does. |
| 9 | CI | The local toolchain (1.97) was older than CI's stable (1.100), which added the `chunks_exact_to_as_chunks` lint; the size commit went red after it was pushed. | 🟡 | Fixed the code (`as_chunks`). CI on latest stable stays the authority, so new lints are seen early; the review checks the CI result of every commit, not only local runs. |
| 10 | case-study method | The first rerun of the comparison ran while a release build and the hostile-image tests used the CPU, so its timings were skewed. It was thrown away and rerun on an idle machine. | 🟡 | Fixed: `compare_engines.py` warns when the load average is above 0.5. |
| 11 | tests | The Phase 1 night-mode bug (dark text on a dark page) survived two phases because no test looked at rendered colours. | 🟡 | Fixed in 3.1 (pixel tests per theme). Rule kept: every visual feature gets a test that checks pixels, not just "it rendered". |
| 12 | corpus | Still only generated PDFs; no Word, InDesign or publisher files. | 🟠 | Carried from Phase 2; Phase 4. |
| 13 | text | Chrome's synthetic bold (Type 3 glyphs) still loses letters in bold Indic headings. | 🟠 | Carried from Phase 2; Phase 4. |

## What holds up

- The lazy page tree removed the slowest thing in the engine (cold open
  333 → 12 ms on CI) with a small, documented change to the vendored parser,
  and it falls back to the full walk for broken trees: every corpus,
  fixture and damaged file gives the same page count and render as before.
- The shared render cache never blocks: a busy cache means a fresh one, so
  a second thread is never made to wait, and a cache left behind by a panic
  is thrown away.
- Themes keep photos in their own colours and treat scanned text like
  text, with a cost that now stays linear whatever the page draws.
- Every number in the case study comes from a script in the repo and is
  rerun before each review.

## Result

🔴 none. 🟠 1 fixed; 2 (size) missed and the target revised with
the reasons; 6, 7, 8, 12, 13 carried to Phase 4 with a plan. 🟡 all fixed or
documented. CI green after the fixes.
