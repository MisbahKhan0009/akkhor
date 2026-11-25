# Phase 3: PDF speed, memory and size (detailed plan)

Goal: a PDF opens instantly whatever its size, zooming and scrolling stay
smooth, night themes work on PDFs, and the Android library gets close to
its size target. Every step is measured before and after.

## Starting point, measured

| Measure | Now | Target |
|---|---|---|
| Night and dusk themes on a PDF | **broken**: the page is filled with the theme's dark background and the PDF's black text is drawn on top, so text is invisible | text in the theme's text colour, images in their own colours |
| Cold open, 2,000-page 308 MB PDF (CI) | 333 ms | under 30 ms |
| Open, same file, warm (desktop) | 6 ms | under 2 ms |
| Last page, whole page at 1x (CI) | 23 ms | unchanged or better |
| One 512×512 tile of a text page at 6x | 8.7 ms, and every tile repeats the whole page's interpretation and glyph outlines | under 4 ms for the second and later tiles of a page |
| Thumbnail of an image page (123×159) | 3.3 ms | under 5 ms for any page |
| Table of contents (PDF outlines) | not available | available, lazy |
| `libakkhor.so`, arm64, all formats | 6.27 MB | under 3.5 MB (see 3.6 for why not 3 MB) |

Why the open is slow: `hayro-syntax` builds the whole page list when the
file is opened, reading the dictionary of every page; on a 2,000-page file
that is 2,000 small reads scattered over 308 MB.

Why tiles repeat work: `hayro` renders a tile by interpreting the whole
page into a canvas the size of the tile, with a fresh `RenderCache` each
time (fonts parsed, glyph outlines built, images decoded again).
`RenderCache` is not `Send` (it holds `Rc`), and `vello_cpu` 0.3 cannot
record a page once and replay it per tile.

Where the size goes (x86_64 build, `cargo bloat`, `.text` 3.7 MB): std
570 KB, hayro-syntax 470 KB, HarfRust 415 KB, read-fonts 385 KB, skrifa
245 KB, JBIG2 220 KB, hayro-interpret 220 KB, JPEG 2000 165 KB, moxcms
155 KB (colour management), fearless_simd 115 KB, zune-jpeg 100 KB; plus
`.rodata` 760 KB (standard PDF fonts, tables) and 530 KB of dynamic
relocations.

## 3.1 Themes on PDFs ("smart night mode")

Every theme maps the page's colours instead of only painting a background:

- The page is rendered as usual on white. Then each pixel outside images
  is remapped by lightness: white becomes the theme's background, black
  becomes the theme's text colour, and colours keep their hue (a red
  heading stays reddish, lighter on dark themes). Paper keeps today's look.
- Images keep their own colours. Their positions come from a light pass of
  the interpreter (a drawing device that records only where images land),
  done once per page and cached with the page.
- Also applies to tiles: the image boxes are in page space, so any tile
  knows which of its pixels are images.

Done when: night, dusk and sepia renders of the corpus pages show text in
the theme colour (checked by pixel counts in a test), and a page with a
photo keeps the photo's colours.

## 3.2 Lazy page tree

The page tree is read on demand:

- the page count comes from the root's `/Count`;
- page N is found by walking down from the root, using each node's
  `/Count` to skip whole subtrees, with inherited attributes (media box,
  rotation, resources) collected on the way; a flat tree whose `/Count`
  equals its number of kids needs one read;
- each page is resolved once and kept;
- a tree that is broken or lies about `/Count` falls back to the full walk
  of today, and a missing tree to today's brute-force search.

This needs a change inside `hayro-syntax` (the page list is private and
built eagerly). It is vendored in `vendor/hayro-syntax` (MIT or
Apache-2.0, like Akkhor) with `[patch.crates-io]`, the change kept small
and documented in `vendor/README.md` so it can be offered upstream.

Done when: cold open of the 2,000-page file is under 30 ms on CI, warm
open under 2 ms, and every corpus and fixture PDF has the same page count
and identical renders before and after.

## 3.3 Reusing work between tiles

- `PdfDocument::render_tiles(page, scale, theme, &mut [(Tile, &mut [u8])])`
  renders several tiles of one page with one `RenderCache`: fonts, glyph
  outlines and decoded images are shared. Android gets
  `renderPdfTiles(handle, page, scale, theme, bitmaps, xs, ys)`.
- The vendored `hayro` is not changed for this; if the measured gain is
  small, the reason is recorded and the batch API stays because it is also
  what makes cancellation (3.4) useful.
- Images: `force_image_interpolation` off at low zoom, on when zoomed in
  (measured for cost).

Done when: second and later tiles of a page cost under half of the first.

## 3.4 Cancellation and memory budgets

- A `CancelToken` (an atomic flag) passed to the batch render: checked
  before each tile, so scrolling past a page stops the remaining tiles.
  Stopping inside one tile is not possible without changing the
  interpreter; tiles are small enough that this is acceptable (measured).
- Every engine cache gets a stated byte budget and a test: page text (4
  pages), recovery tables (per process, bounded by registered fonts), image
  boxes per page (bounded LRU), render caches (dropped after each batch).
- The app keeps finished tile bitmaps; the engine documents the budget it
  should use.

## 3.5 Thumbnails and table of contents

- `PdfDocument::thumbnail(page, max_side)` renders at the scale that fits,
  with anti-aliasing kept and images decoded at reduced size where the
  decoder allows it.
- `PdfDocument::outline()` reads `/Outlines`: title, level, target page
  (resolving `/Dest`, named destinations and `GoTo` actions), lazily and
  with a cycle guard. Titles in PDFDocEncoding or UTF-16 are decoded.
  Android: `outline(handle)` returning titles, levels and pages.

## 3.6 Library size

In order of expected gain, each measured on arm64 in CI:

1. Packed relocations (`-Wl,--pack-dyn-relocs=android`, supported from
   Android 6, our minimum is 7): most of the 530 KB of relocations.
2. `opt-level = "s"` for the crates off the hot path (parsers of rare
   formats, colour management), `3` for the rasterizer and shaper; render
   time checked so nothing hot slows down by more than 5%.
3. Format switches inside the vendored `hayro-syntax`: JBIG2, JPEG 2000
   and CCITT become separate features (on by default: scanned books use
   them), so a reader that does not need them saves about 400 KB.
4. The 14 standard PDF fonts loaded from a file the app ships (or the
   phone's fonts) instead of compiled in.

The original 3 MB target was set before the PDF text recovery needed a
shaper and font parser in the PDF build too; HarfRust, read-fonts and
skrifa are about 1 MB of code that the reader needs anyway. The target
becomes 3.5 MB for all formats, under 3 MB for a PDF-only build without
the rare image codecs.

## 3.7 Case study material: comparison with other engines

For a public write-up (a LinkedIn post and a case study on the author's
website), every claim needs data and a figure that anyone can reproduce.
Everything lives in `docs/case-study/`:

| Path | What |
|---|---|
| `scripts/bench/compare_engines.py` | runs Akkhor, MuPDF (PyMuPDF), PDFium (pypdfium2, Chrome's engine) and Poppler (`pdftoppm`, `pdftotext`) on the same files and writes the data |
| `scripts/bench/make_figures.py` | turns the data into charts (SVG for the web, PNG for posts) |
| `crates/akkhor-cli` `--json` | Akkhor's own timings, measured inside the process like the others |
| `docs/case-study/data/` | results as CSV and JSON, with the machine, versions and date |
| `docs/case-study/data/history.csv` | one row per milestone and metric, from Phase 0 on, so progress can be charted |
| `docs/case-study/figures/` | the charts and screenshots |
| `docs/case-study/case-study.md` | long-form draft for the website, with every number linked to its data |
| `docs/case-study/linkedin-post.md` | post draft with the images to attach |

Comparisons, each with its method and caveats written next to the numbers:

1. **Bangla and Hindi text accuracy** (the headline): character error rate
   of the text each engine extracts from the 36 corpus PDFs, by language,
   producer, with and without ToUnicode, and Bijoy. Plus "search
   success": the share of corpus words each engine's text lets you find.
2. **Side-by-side sample**: the same sentence as copied out of the same
   PDF by each engine.
3. **Open time** on the 2,000-page, 308 MB PDF (warm, and cold where the OS
   allows dropping caches) and on small files.
4. **Render time**: first page and a deep page at 1x and 2x.
5. **Memory**: peak resident memory to open the big file and render its
   last page.
6. **Library size** on the same machine (x86_64, stripped) and Akkhor's
   Android sizes per architecture.
7. **Robustness**: damaged and random files survived, per engine where they
   can be tried the same way.
8. **Themes**: screenshots of one page in every theme.
9. **Progress over the phases**: open time, text error rate, library
   size, from `history.csv`.

Fairness rules: in-process timings for the libraries (no interpreter or
process start-up), Poppler measured from its command line with the start-up
cost of `pdftoppm -v` subtracted and marked as such; best of 5 runs after
one warm-up; same scale and page for everyone; versions recorded. Our
corpus is generated by us, and that is said wherever it is used.

From now on every phase adds its measurements to `history.csv` and
refreshes the data and figures before its review.

## 3.8 Benchmarks and review

The CI benchmark gains: cold and warm open, first and second tile,
thumbnail, outline, night render, sizes per build. Then the rigorous
review of Phase 3, as for Phases 1 and 2.

## Done when

- [x] Night, dusk and sepia work on PDFs; images keep their colours (cost:
  about 2 ns a pixel, +1.2 ms for a 512×512 tile)
- [x] Lazy page tree; identical page counts and renders on every corpus
  and fixture PDF (`scripts/compare_renders.sh`). Warm open of the
  2,000-page PDF 12 ms → 0.5 ms, heap after open 6.6 → 1.0 MB, file touched
  at open 298 → 7 MB (desktop). Cold open on CI: see the benchmark.
- [x] Second and later tiles of a page under half the cost of the first
  (512×512 tiles of a Bangla/Hindi page at 4x: first 6.1 ms, then about
  1.1 ms; drawing a page again 5.4 → 2.9 ms). One render cache per
  document, never waited for (a busy cache means a fresh one), started over
  after 16 pages because hayro's cache only grows.
- [x] Batch rendering can be cancelled between tiles (`render_tiles`,
  Android `renderPdfTiles` and `cancelRendering`, which never waits for the
  render it stops)
- [x] Memory over a reading session measured (`akkhor-bench session`): the
  heap grows about 7 KB per page read (each page keeps its decoded
  content, as upstream hayro does). `PdfDocument::trimmed()` and Android
  `trimMemory` reopen the file (under 1 ms) and shed it: 2.1 → 1.1 MB after
  200 pages.
- [ ] Thumbnails and table of contents, in the CLI and on Android
- [ ] arm64 library under 3.5 MB with all formats
- [x] Case study data, figures and drafts in `docs/case-study/`,
  reproducible with two scripts (first run 2026-11-25; it also exposed the
  overstated Phase 2 baseline and led to `/ActualText` support)
- [ ] Review written and its findings fixed; CI green

## Risks

- Vendoring `hayro-syntax` means following upstream by hand. Kept to the
  page tree and feature switches, with a written diff, and offered
  upstream.
- Lightness remapping can make some designed colours look different from
  the original. Paper theme keeps the original look; a "keep original
  colours" option stays available for any theme.
