# Akkhor: a reading engine that gets Bangla and Hindi right

*Draft for the website case study. Every number links to its data in
[`data/`](data/); figures are in [`figures/`](figures/). Measured on
2026-11-25 on a 2-core cloud VM (`data/environment.json`).*

## In one paragraph

Akkhor is a small, fast engine for reading books on Android, written in
Rust. It opens PDF, EPUB and plain text, and it was built around one
question most reading engines treat as an afterthought: what happens to
Bangla and Hindi text when you copy it, search it, select it or read it
aloud. On PDFs that carry no usable text information, established engines
return 97 to 100% wrong characters; Akkhor returns the exact text when it
knows the font. Bangla typed in the old Bijoy encoding comes out as
gibberish from every other engine and as correct Unicode from Akkhor. And
it opens a 2,000-page, 308 MB PDF in 0.16 ms when the file is cached and
3 ms straight from disk.

## The problem

Bangla and Devanagari are written in syllables. A conjunct such as ক্ষ
(k + ṣ) or স্ত্র (s + t + r) is drawn as one shape, a vowel sign like ি
is drawn *before* the consonant it follows, and the reph (র্) is drawn
*after* the syllable it starts. A PDF stores the shapes, not the letters,
so getting the letters back depends on extra information the producer may
or may not have written, and on the reader interpreting it well.

What readers see in practice:

- copied text with stray spaces in the middle of words, so search misses
  them ([`figures/copied-text-samples.png`](figures/copied-text-samples.png));
- PDFs with no text information at all, from older tools, where copy and
  search return nothing useful;
- a large amount of Bangla still typed in **Bijoy**, a pre-Unicode
  encoding that only looks like Bangla in a Bijoy font. Every other engine
  hands back `evsjv fvlvi` for `বাংলা ভাষার`.

On top of that, the common engines are heavy for a phone: MuPDF's library
is 36 MB on Linux, and none of them lays out EPUB text with Indic shaping
and syllable-aware selection in the same package.

## Goals

1. Exact Bangla and Hindi text for copy, search, selection and read-aloud,
   in PDFs, EPUBs and text files, including Bijoy.
2. Light: memory-mapped files, nothing read until it is needed, each
   format a build-time switch.
3. Fast on big files: open instantly, jump to any page, render tiles at
   any zoom straight into the phone's bitmap.

## How it works

- **One engine, format switches.** PDF, EPUB and TXT are Cargo features;
  an app that only reads PDFs compiles nothing else. Files are
  memory-mapped, so a 308 MB PDF costs a few MB of memory.
- **Reflowable text** (EPUB, TXT) goes through a layout pipeline with
  HarfRust shaping, Liang hyphenation and syllable-aware hit-testing,
  selection and search, so a selection never splits a conjunct.
- **PDF rendering** uses `hayro`, a pure-Rust PDF interpreter, with my
  changes around it: tiles drawn straight into the caller's buffer, reading
  themes that keep images in their own colours, and a lazy page tree (see
  below).

### Getting the letters back from PDF glyphs

This is the core of the project. For every font embedded in a PDF:

1. **Find the full font.** PDF producers embed a stripped subset: no
   shaping rules, sometimes renumbered glyphs, sometimes a character map
   filled with junk. But a glyph's *outline* survives. Akkhor hashes every
   embedded glyph's outline and looks it up in complete copies of fonts it
   knows: its bundled Noto fonts and the phone's own fonts.
2. **Run the font's rules backwards.** In the complete font, Akkhor shapes
   candidate syllables (consonant + vowel sign, reph + consonant,
   consonant pairs, three-consonant clusters, guided by the font's own
   substitution tables) and records which glyph each one produces. A glyph
   that only appears for স্ত্র *means* স্ত্র.
3. **Put the text in reading order.** Vowel signs drawn first move after
   their consonant cluster, rephs move to the start of their syllable,
   split vowels (ে … া) are rejoined, and a reph fused with a vowel sign is
   split in two directions at once.
4. **Use what the producer wrote, when it is there.** Chrome and
   LibreOffice also write `/ActualText` spans with the correct text of
   each cluster; Akkhor reads those first.

Bijoy runs (fonts named like SutonnyMJ) are converted with a table derived
from the MIT-licensed `poriborton` crate, then reordered the same way.

## Results

### Text you can copy and search

![Characters wrong per engine](figures/text-accuracy.png)

Character error rate on 36 PDFs (Bangla and Hindi prose and full conjunct
lists, two producers, four fonts), from
[`data/text_accuracy.csv`](data/text_accuracy.csv):

| | PDFs as produced | No text layer | Bijoy |
|---|---|---|---|
| **Akkhor (+ device fonts)** | **0.0%** | **0.0%** | **0.0%** |
| Akkhor (bundled fonts only) | 0.0% | 41.0% (0.0% on Noto, 82% on unknown fonts) | 0.0% |
| MuPDF 1.28.6 | 0.5% | 99.8% | 96.5% |
| PDFium 156 | 0.6% | 99.7% | 96.4% |
| Poppler 24.02 | 0.0% | 99.7% | 96.4% |

On well-made PDFs everyone gets the characters nearly right. The
difference shows up in **search**: Akkhor keeps every word whole, while
the others break some words with stray spaces.

![Words found by search](figures/search.png)

| Words found | Akkhor | MuPDF | PDFium | Poppler |
|---|---|---|---|---|
| 16 PDFs as produced | **100%** | 96.6% | 86.4% | 97.1% |

![Copied text samples](figures/copied-text-samples.png)

### Speed on a very big PDF

![Open and render the last page of a 2,000-page PDF](figures/speed-big-pdf.png)

2,000 pages, 308 MB ([`data/speed.csv`](data/speed.csv), best of 5):

| | Open (warm) | Last page, 1x | Last page, 2x | Open from disk (cold) |
|---|---|---|---|---|
| **Akkhor** | **0.21 ms** | **2.7 ms** | **4.5 ms** | 3.0 ms |
| MuPDF | 0.50 ms | 12.8 ms | 12.4 ms | 5.8 ms |
| PDFium | 0.97 ms | 15.8 ms | 14.4 ms | **2.6 ms** |
| Poppler | 1.7 ms* | 23.9 ms* | 36.1 ms* | n/a |

\* Poppler measured through its command-line tools, start-up subtracted.
Every library renders four bytes a pixel (RGBA, as Android bitmaps are);
Poppler's `pdftoppm` uses three.

From a cold disk, opening the file *and* drawing its last page takes Akkhor
13.5 ms, MuPDF 219 ms and PDFium 185 ms
([`data/cold_open.csv`](data/cold_open.csv)): they read far more of the
file to find page 2,000.

The open time came from a change I made to the PDF parser: it used to read
the dictionary of every page when the file was opened. Reading the page
tree lazily took the warm open from 12.1 ms to 0.46 ms, the memory after
opening from 6.6 MB to 1.0 MB, and the share of the file pulled into memory
from 298 MB to 7 MB.

![Lazy page tree, before and after](figures/progress-lazy-page-tree.png)

On the CI machine, opening the same file straight from disk went from
333 ms to 12 ms.

### Phase 3 in four numbers

![Phase 3 before and after](figures/progress-phase-3.png)

- **Cold open** of the 2,000-page PDF on CI: 333 → 12 ms.
- **Redrawing** a Bangla and Hindi page at 2x: 5.4 → 3.0 ms. One render
  cache is now shared by every page and tile of a document, so fonts,
  glyph outlines and images are parsed once. Zoomed tiles of a page cost
  about 1.1 ms each after the first (6.1 ms).
- **A long reading session** grows the heap by about 7 KB a page; when
  Android asks apps to trim memory, Akkhor reopens the file in under 1 ms
  and drops back from 2.1 to 1.1 MB.
- **The Android library** (arm64): 6.42 → 5.81 MB, with scan decoders
  (JBIG2, CCITT, JPEG 2000) now a switch. A PDF-only build is 4.97 MB,
  EPUB and text only 2.19 MB.

### Reading themes

PDFs are designed as dark ink on white paper. Akkhor redraws the page in
the reader's theme by lightness: paper becomes the theme's background, ink
its text colour, colours keep their hue, and colour images keep their real
colours. Scanned pages (greyscale images) follow the theme like text.

![One page in five themes](figures/themes.png)

### Robustness

![200 damaged PDFs](figures/robustness.png)

200 damaged PDFs, the same files for every engine: **no engine crashed or
hung**. Akkhor shows page 1 of the most damaged files (145), then MuPDF
(130), Poppler (117) and PDFium (109); the rest are refused with an error.
Two of those 145 came in Phase 4b: a single flipped byte inside a number of
a page's `/MediaBox` used to make the whole page dictionary unreadable;
the parser now skips the bad token, as MuPDF does.
"Opened" means a page was shown: my first run also counted files that
MuPDF and PDFium opened with zero pages, which gave MuPDF 180. With the
same rule for everyone, that lead was an artefact.

### Reading a PDF like an EPUB

![A two-column PDF page and its reflow on a phone](figures/reflow-showcase.png)

A PDF page is made for paper. On a phone, a two-column A5 page means
zooming and panning line by line. Akkhor can **reflow** it: the page's text
becomes headings and paragraphs in reading order, set by the same layout
as EPUBs at any size and theme, with pictures kept.

Getting that right takes more than reading lines top to bottom:

- **Columns**: two columns drawn row by row are read column by column, and
  a heading over both columns comes first.
- **Running headers and page numbers** (in Bangla or Devanagari digits
  too) are left out; producers that tag them (Typst, LibreOffice) are
  believed, the rest are found because they repeat on nearby pages.
- **Paragraphs** continue across columns and pages; line-end hyphens are
  removed, except in compounds (পাঠ-ইঞ্জিন) and doubled words (धीरे-धीरे).
- **Footnotes** follow their page in a smaller size.
- Every position maps back to the page, so the reader can switch between
  the page view and the reflow and stay at the same sentence.

Measured on 22 book-layout PDFs made by Chrome, Word (through
LibreOffice), XeLaTeX and Typst, in Bangla and Hindi
([`data/books.csv`](data/books.csv)): 0.0% wrong characters in the
reflowed text, 100% of paragraph and heading breaks found, every heading at
the right level, every footnote found, no running header or page number
left in. Analysing a 16-page chapter takes under 1 ms once its text is read.

The same corpus also shows how often the other engines break words in book
PDFs:

![Words that come out whole, per PDF maker](figures/books-words-intact.png)

XeLaTeX draws no space characters and squeezes the gaps in justified
lines; MuPDF keeps 5% of words whole there, Akkhor 99.9% (it measures the
gaps line by line).

### A real book

![A real Bangla book: words intact](figures/real-world-book.png)

Generated test files are cleaner than real ones. A 200-page Bangla book
made in Word and passed through Acrobat and Ghostscript (copyrighted, so
only its numbers are published) exposed three problems that no generated
file had: bold text drawn twice on itself (every letter came out doubled),
font programs rewritten so their outlines moved sideways, and a body font,
Kalpurush, that the book uses without any text information for its
conjuncts. All three are fixed and now reproduced on our own test files.
With Kalpurush registered (it is free, under the OFL), 100% of the book's
Bangla words come out whole; with only the bundled fonts, 68%; MuPDF, PDFium
and Poppler about 26%.

### Tables and cards

![A page with tables and a card, and its reflow](figures/tables-showcase.png)

Textbooks, word lists and exam papers are full of tables and boxes, and a
reflow that reads them line by line, or down their columns, is useless.
Phase 4b finds them before deciding the reading order:

- **Tables** with ruled grids (Word's "Table Grid", HTML borders), with
  horizontal rules only (LaTeX's booktabs), or with no rules at all, where
  the cells line up. Their cells are never mistaken for columns.
- **Small tables (up to three columns) become text**, one paragraph per
  row with the cells in order (শব্দ · অর্থ · উদাহরণ), so they read aloud
  and search like any sentence, and every word still maps back to its cell
  on the page. **Wider ones become a picture** of that part of the page,
  with their text kept for read-aloud.
- **Cards and shaded boxes** are read box by box: three cards side by side
  are three cards, not three columns read line by line across.
- A bold line ending in a colon introduces what follows and is not a
  heading; centred lines of different lengths are not taken for new
  paragraphs.

![Tables and cards per engine](figures/tables.png)

On 8 new test PDFs (Chrome, Word through LibreOffice, XeLaTeX, Typst;
Bangla and Hindi; [`data/tables.csv`](data/tables.csv)) every table row
comes out whole and in order and every card stays together. The other
engines have no reflow, so they are scored on their page text, the order a
screen reader gets: MuPDF keeps 65% of table rows in order, PDFium 76%,
Poppler 48%. The test files are mine and made for this; the real book
below is the check that the rules are not just fitted to them.

On the real Bangla book (not published,
[`data/real_world_tables.csv`](data/real_world_tables.csv)) its tables now
reflow as 585 rows on 53 pages, cells in order, where Phase 4 read each
table down its columns; and the reflow has 356 headings instead of 1,221,
because row numbers, labels around pictures and lead-ins ending in a colon
no longer count.

### Ready for an app

Phase 5 put everything a reader app needs behind one interface: the
library card (title, authors, cover, how sure the text is), a saved place
that survives any change of font, screen or theme, a progress bar,
search of the whole book that the app can stop at any time, highlights,
sentences for read aloud, and "share these pages" as a new PDF.

- **One Kotlin file** (`android/Native.kt`, 54 calls) is the whole
  contract. CI checks every declaration against the library and calls
  each one from a JVM, from several threads, after closing the book and
  with bad input, so a mismatch fails the build, not the app.
- **Shared pages stay real PDF pages.** Pages from all 79 test PDFs,
  copied and opened in MuPDF, PDFium and Poppler, give the same text and
  the same pixels as the original ([`data/page_copies.csv`](data/page_copies.csv)).
- **Fuzzing** (eight targets, every push) and the review found four bugs
  before any app did: a parser hang my own Phase 4b change had caused, an
  EPUB panic on an odd link, a deadlock when a book is closed during a
  search, and pages turned by `/Rotate` read one letter per line
  ([`data/phase5_fixes.csv`](data/phase5_fixes.csv)).

### On a real phone

On a Realme 9i, a mid-range phone (Snapdragon 680, Android 13), any book
opens in under a millisecond, a PDF page at the phone's full width draws
in about 30 ms, a reflowed screen in about 17 ms, and a chapter lays out
in a few milliseconds, with 14 to 21 MB of memory added
([`data/phone.csv`](data/phone.csv), [`data/phone_summary.csv`](data/phone_summary.csv)).
The phone is two to three times slower than the cloud VM the other
numbers come from. One cost stands out: the first Bangla or Hindi PDF
of a run spends up to 0.4 s building tables from the reference fonts,
once; keeping them on disk is next.

## Where Akkhor is behind (and what is next)

Being honest about this is part of the point:

- **Drawing the same page again.** Sharing the render cache took Akkhor
  from 5.4 to 2.3 ms on the Bangla and Hindi page, ahead of PDFium (2.5 ms)
  but behind MuPDF (1.6 ms), which records a page once and replays it
  ([`figures/speed-redraw.png`](figures/speed-redraw.png)). The renderer
  Akkhor uses cannot record a page yet.
- **Memory on an Indic page at 2x:** 11.6 MB added against 9.0 to 10.7 MB
  for the others ([`figures/memory.png`](figures/memory.png)); the
  difference is a constant few MB (code and parsed fonts), not growth per
  page.
- **Unknown fonts with no text layer:** if neither the PDF nor the phone
  has the font, there is nothing to recover from.
- **Tables:** a header row whose labels sit in coloured boxes is read as
  one line above the table, not as its header; merged cells send a table to
  a picture rather than text; a table in a font with no bold has a plain
  header row.
- **Library size:** 6.0 MB on Android (arm64) for the app build (all
  formats, scans, the page writer and Indic shaping): smaller than PDFium (6.7 MB, PDF only) and far
  below MuPDF (36 MB), but I missed my 3.5 MB target. Most of what is left
  is the shaper and font parsers, which both EPUB layout and PDF text
  recovery need.

## What I got wrong along the way

- I first assumed embedded fonts keep their shaping rules. Measuring showed
  both Chrome and LibreOffice strip them, which led to the
  reference-font design above.
- My first "before" numbers (31 to 45% wrong text) came from reading each
  glyph's `ToUnicode` entry the way the PDF library exposes it. Comparing
  with MuPDF, PDFium and Poppler showed they also read the `/ActualText`
  spans the producers write and get under 1%. I corrected the docs, added
  `/ActualText` support to Akkhor, and rebuilt the "no text layer" test set
  without those spans so it really tests recovery from glyphs.
- Night mode on PDFs was broken from the start (dark text drawn on a dark
  background) and no test caught it; I only found it when I measured
  everything before planning Phase 3. A render test now checks the colour
  of text, background and images in every theme.
- I set a 3 MB, then 3.5 MB, size target before knowing what Indic text
  recovery would need. Measuring every build showed the floor; the target
  is now 5.5 MB and the reasons are in the Phase 3 review.
- My first robustness run counted a damaged file "opened" when the
  library opened it, even with zero pages. MuPDF opens many truncated
  files that way, so it looked far ahead (180 to 143). Counting only files
  that show a page, Akkhor leads (143 to 130).
- The review of the night theme found a page that draws 100,000 small
  images made it 450 ms slower, because every pixel was checked against
  every image. A row-by-row sweep brought that to 40 ms.

- My Phase 4 reflow numbers were all from generated books with no tables.
  The first time I counted headings in the real book's reflow (1,221 for
  204 pages) was in Phase 4b; most were row numbers and labels.

- My Phase 4b fix for damaged arrays (skip a broken element, go on) made
  an unterminated nest of arrays exponential. Fuzzing found it in under
  two minutes; no hand-made test had nested broken arrays.

## How it was built

- **A plan before every phase**, pushed to GitHub before any code, and a
  written, rigorous review after every phase with each finding fixed or
  carried forward ([`docs/reviews/`](../reviews/)).
- **A measured corpus**: 36 PDFs, a Bijoy round trip over 3,842 words,
  checked in CI on every push.
- **Robustness tests** in CI: hundreds of damaged files driven through every
  parser, renderer and the text layer.
- **This comparison**, reproducible with two scripts.

## Method, in short

Every measurement runs in a fresh process. Library calls are timed inside
the process; Poppler through its command-line tools with start-up
subtracted. Character error rate is edit distance over the length of the
source text after Unicode decomposition, whitespace ignored. Full details
and caveats: [`data/README.md`](data/README.md).
