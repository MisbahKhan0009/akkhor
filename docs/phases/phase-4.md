# Phase 4: Indic PDF reflow (detailed plan)

Goal: any text PDF can be read like an EPUB on a phone screen: the page's
text becomes paragraphs and headings in reading order, without running
headers, page numbers or line-end hyphens, laid out by the same pipeline
as EPUBs, with positions that map both ways so the reader can switch
between the original page and the reflowed text at the same spot. Plus the
items carried from the Phase 2 and Phase 3 reviews.

## Starting point, measured (2026-11-25)

Probe PDFs made from the corpus text in book-like layouts (A5, justified,
two columns, running header, page numbers, a footnote, hyphenation):

| Probe | Akkhor today | MuPDF |
|---|---|---|
| Chrome, two CSS columns, hyphenated | 100% of words found; columns in the right order; line-end hyphens kept (`শি-` / `ক্ষার্থীরা`) | 90.0% |
| **XeLaTeX** (fontspec, two columns, fancyhdr) | **91.3%**: in tightly justified lines words run together (`কিংবাস্ত্র।পাঠকযখনকোনোবইখোলেন,`) because XeLaTeX draws no space glyphs and the gap test (0.22 em) is too coarse | 17.3% |
| Word document (pandoc .docx → LibreOffice) | 100% | 32.7% |
| Running header and page number (XeLaTeX) | read as a text line (`অক্ষর পরীক্ষা 1 প্রথম অধ্যায়`) | same |
| LuaLaTeX with HarfBuzz | not produced yet (set-up) | |

There is no reflow: a PDF can only be shown as its pages. The text layer
already gives lines with boxes, word gaps, blank-line paragraph guesses and
reading order as drawn (`PageText`), and the image boxes from the theme
pass give where pictures are.

## 4.1 A book-like corpus with ground truth

The Phase 2 corpus is single-column prose; reflow needs book layouts and
more producers. `scripts/make_corpus.py --books` generates, from the same
source texts (whose paragraphs and headings are known):

- producers: Chrome, LibreOffice Writer (from `.docx` and `.odt`: the path
  Word documents take), **XeLaTeX** and **LuaLaTeX** (fontspec; the
  HarfBuzz renderer), and Typst if it installs from PyPI;
- layouts: one and two columns, justified and ragged, running headers and
  page numbers, footnotes, line-end hyphenation, headings at two levels,
  a figure, a paragraph that continues onto the next page;
- Bangla and Hindi, two fonts each, as in Phase 2.

Every generated PDF has a ground-truth JSON beside it: the blocks
(heading or paragraph) in reading order. Measured per file:

- **words found** (as in the case study) on the page text;
- **reflow character error rate**: the reflowed text against the ground
  truth, blocks joined by a blank line;
- **block boundaries**: precision and recall of paragraph and heading
  starts; headings found with the right level;
- **furniture removed**: running headers, footers and page numbers that
  leak into the reflow (target 0).

Real-world PDFs (Word, InDesign, publishers) cannot be generated. A
`tests/real/` folder takes PDFs the author has permission to use, each
with a small hand-checked ground truth (a few paragraphs per file); CI
measures whatever is committed there. Collecting them needs the author.

## 4.2 Word gaps

Spaces a PDF does not draw are inferred from gaps. Today a gap over 0.22
em is a space; XeLaTeX and InDesign justify with smaller gaps.

- Use the font's own space width when it has one (`/Widths` entry for the
  space, or the font program's space advance) and call a gap a space when
  it exceeds a share of it (about 40%).
- Per line, look at the gap distribution: in a justified line inter-word
  gaps are many and similar, kerning gaps small. A clear two-cluster split
  sets the threshold for that line.
- Never split inside a syllable: a gap before a mark, virama or vowel sign
  is never a space.

Done when: 100% of words found on every XeLaTeX and LuaLaTeX corpus PDF,
and no corpus PDF gets worse (CI checks words found and CER).

## 4.3 Page structure

A new module `akkhor-pdf::structure` turns a page's lines into blocks.
Input: the lines of `PageText` with their boxes, font sizes and font names,
plus the image boxes. Output per page: blocks in reading order, each a
heading (level), paragraph, image, footnote or furniture, with the page
text ranges it covers.

1. **Columns.** Lines are grouped into regions separated by vertical
   white space; a region with a tall white gutter is split into columns.
   Order: full-width regions top to bottom, columns left to right inside
   each (the drawing order is kept when it already agrees).
2. **Furniture.** A line in the top or bottom band of the page whose text,
   digits ignored, repeats on neighbouring pages, or a line that is only a
   page number (Latin, Bangla or Devanagari digits, roman numerals), is
   furniture: kept in the page text, left out of the reflow.
3. **Paragraphs.** A new paragraph starts at a first-line indent, at a
   vertical gap larger than the region's line spacing, or after a line
   that ends well short of the column's right edge (with justified text).
4. **Headings.** Lines larger than the page's body size (the most common
   size, by character count), or bold and short and set apart; levels by
   size rank across the chapter. Titles in the PDF outline confirm them.
5. **Line-end hyphens.** A line ending in `-`, U+2010 or U+00AD followed by
   a line starting with a letter is joined without the hyphen, unless the
   hyphenated form appears inside a line elsewhere in the chapter (a real
   compound such as `পাঠ-ইঞ্জিন`).
6. **Footnotes.** Small text at the bottom of a page, below the body, often
   after a short rule: kept as small paragraphs at the end of the page's
   blocks.
7. **Styles.** Bold and italic spans from the font name (`Bold`,
   `Semibold`, `Italic`, `Oblique`) or a synthetic-bold stroke.
8. **Images.** Image boxes larger than a small share of the page become
   image blocks, placed by their position in the reading order.

Pure functions over data, so every rule is unit-tested on hand-made lines,
and the corpus measures the whole.

## 4.4 Reflow

- `PdfDocument::reflow_chapters()`: chapters from the outline's top level
  when it has one, else runs of 16 pages, cut at a page boundary.
- `PdfDocument::reflow(chapter) -> Reflow { document: Document, map }`:
  analyses only that chapter's pages (cached like page text), joins a
  paragraph that continues across a page break, drops furniture.
- `map` turns a reflow position `(block, offset)` into a PDF position
  `(page, offset)` and back, so selection, search, bookmarks and
  read-aloud work in either view and switching views keeps the place.
- Image blocks point at a page region; an `ImageProvider` for PDFs renders
  that region (original colours) at the size the layout asks for.
- The flow layout, hyphenation, selection and search are reused unchanged.

Done when on the book corpus: reflow CER under 0.5% (target 0%), block
boundaries precision and recall over 95%, no furniture leaked, every
two-column file in order, hyphens joined; a 16-page chapter is analysed in
under 50 ms on the desktop once its page text is built.

## 4.5 Interfaces

- CLI: `akkhor book.pdf --reflow [--chapter N] [--size WxH] --out page.png`
  and `--reflow --text` to print the reflowed text.
- `akkhor` crate: `Book::Pdf` reflow chapters behave like EPUB chapters.
- Android: `reflowChapterCount(handle)`; `layoutChapter`, `renderFlowPage`,
  `hitTest`, `selectionRects`, `textBetween` and `search` work on a PDF's
  reflow chapters; `pdfToReflow(handle, pdfPos)` and
  `reflowToPdf(handle, chapter, pos)` convert positions.
- UTF-16 offsets for read-aloud (carried from Phase 2): `toUtf16Offset`
  and `fromUtf16Offset` on text the app already holds.

## 4.6 Bold headings drawn as Type 3 glyphs

Chrome draws synthetic bold as Type 3 fonts: each glyph is a small content
stream of paths, with no font program and no `ToUnicode`. Their outlines
are the regular glyph's outline (stroked or offset for weight). Plan: hash
the Type 3 glyph's fill path after mapping it through the font matrix, the
same way embedded glyphs are hashed (Phase 2), and match against the
reference fonts; if the stroke changes the path, match on the path before
stroking. Done when the bold headings of `indic.pdf` read exactly.

## 4.7 Carried from the Phase 3 review

In priority order, each measured before and after, each stopping if it
does not pay:

1. **Library size**: the 14 standard PDF fonts loaded from a file instead
   of compiled in (0.27 MB), a `cargo bloat` pass on the arm64 build,
   HarfRust tables the engine never uses. Target: under 5.5 MB all
   formats, under 4.5 MB PDF only.
2. **Damaged files**: find why the 57 refused damaged files fail (most are
   expected to be a broken cross-reference table) and rebuild the table by
   scanning the file for objects, as MuPDF does. Target: within 10 files
   of MuPDF, still no crash or hang.
3. **Memory on Indic pages**: parse each embedded font once and share it
   between text recovery and rendering. Target: within 1 MB of MuPDF on
   the Indic page at 2x.
4. **Redraw**: a study only: can a page be recorded once and replayed per
   tile with `vello_cpu` 0.3, or must it wait for a newer version. Written
   up with measurements; implemented only if the gain is over 2x.

Moved out of Phase 4: the downloadable pack of Bangla and Hindi fonts
(belongs to the app, needs licence checks per font), Kruti Dev (no
verifiable table yet).

## 4.8 Benchmarks, case study and review

The CI benchmark gains reflow of a chapter (time and memory) and the book
corpus scores. The case study gains a reflow section (a two-column XeLaTeX
page, the original next to the reflowed phone page in night theme) and the
comparison gains the book corpus for every engine's text. `history.csv`
gets the Phase 4 rows. Then the rigorous review.

## Done when

- [x] Book corpus with ground truth: 22 PDFs from four producers (Chrome,
  Word via LibreOffice, XeLaTeX, Typst; LuaLaTeX scripted but unavailable
  here); `tests/real/` ready for permitted PDFs, a real book measured
  locally
- [x] Words intact on XeLaTeX PDFs 89.4% → 100% (Mukta 99.5%); nothing
  worse
- [x] Page structure: columns, furniture, paragraphs, headings, hyphens,
  footnotes, styles, images, each unit-tested
- [x] Reflow with a two-way position map; targets in 4.4 met on the corpus
  (CER 0.0%, block starts 100%, no furniture, under 1 ms a chapter)
- [x] CLI, `akkhor` crate and Android interfaces; UTF-16 offsets
- [x] Type 3 bold headings read exactly, also with no text layer at all
- [x] Carried items measured and done or stopped with reasons: size
  5.73 MB arm64 (target 5.5 missed by 0.23), damaged files (Akkhor leads
  once the rule is fair), memory (constant overhead, not per page), redraw
  (needs a renderer that can record)
- [x] Case study, figures and `history.csv` updated; review written
  (`docs/reviews/phase-4.md`) and its findings fixed or carried; CI green

## Risks

- Layout analysis is heuristic. Every rule is measured on generated
  books, which are cleaner than real ones; the real-world folder is what
  keeps it honest, and it needs PDFs with permission.
- Furniture detection can drop a real line that repeats (a refrain, a
  repeated heading). Only lines in the top and bottom bands are
  candidates, and only when they repeat on neighbouring pages.
- Joining hyphens can merge a real compound. The rule keeps a hyphen that
  appears inside a line anywhere in the chapter; misses are counted in the
  corpus score.
