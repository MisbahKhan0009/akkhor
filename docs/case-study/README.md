# Akkhor case study material

Everything needed for a write-up about Akkhor: drafts, figures and the data
behind every number, all reproducible.

| | |
|---|---|
| [`case-study.md`](case-study.md) | long-form draft for the website |
| [`linkedin-post.md`](linkedin-post.md) | post draft and the images to attach |
| [`figures/`](figures/) | charts (SVG for the web, PNG for posts) and screenshots |
| [`data/`](data/) | the measurements, the machine and versions, and how each was taken |

## Figures

| File | Shows |
|---|---|
| `text-accuracy` | characters wrong in text copied out of Bangla and Hindi PDFs, per engine: PDFs as produced, PDFs with no text layer, Bijoy PDFs |
| `search` | share of words you can find by searching the copied text |
| `copied-text-samples.png` | the same line as copied out by each engine |
| `speed-big-pdf` | open a 2,000-page 308 MB PDF and render its last page |
| `speed-indic-pages` | first render of a Bangla and Hindi page |
| `speed-redraw` | drawing the same page again (where Akkhor is behind) |
| `cold-open` | opening the big PDF straight from disk |
| `memory` | memory added to open a PDF and render one page |
| `size` | library sizes (not like for like; see the note on the chart) |
| `robustness` | 200 damaged PDFs: opened, refused, crashed |
| `progress-lazy-page-tree` | Phase 3.2 before and after |
| `reflow-showcase` | a two-column XeLaTeX page and three phone screens of its reflow (night theme) |
| `books-words-intact` | words that come out whole, per engine and PDF maker (book corpus) |
| `real-world-book` | a real Bangla book: words intact per engine |
| `progress-phase-3` | Phase 3 before and after: cold open, redraw, session memory, Android size |
| `themes.png` | one PDF page in the five reading themes |

## Reproduce

```sh
cargo build --release -p akkhor-cli -p akkhor-android
python3 scripts/make_fixtures.py --big 2000 --image-kb 150   # the 308 MB PDF
pip install pymupdf pypdfium2 matplotlib                    # plus poppler-utils
python3 scripts/bench/compare_engines.py                    # writes data/
python3 scripts/bench/make_figures.py                       # writes figures/
```

`compare_engines.py --only text,speed` runs part of it; `--quick` uses
fewer runs. Every phase from Phase 3 on adds its measurements to
`data/history.csv` and refreshes the data and figures before its review.
