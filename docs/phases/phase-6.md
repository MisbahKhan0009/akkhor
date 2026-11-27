# Phase 6: every format, and the engine finished for the app

Status: planned (2026-11-27). Misbah's call: finish the engine before the
reader app, so the app is written once against an engine that does not
change under it. This phase adds MOBI/AZW3, FB2 and CBZ behind the same
calls as EPUB, PDF and TXT, and clears the engine items Phase 5 carried.
What the app's screens need beyond formats (layout options, links,
TalkBack paragraphs, error codes and more) is Phase 6b, also before the
app. More scripts stay in Phase 7, release work in Phase 8; neither
changes what the app calls.

## Starting point (2026-11-27)

| Needed | Today |
|---|---|
| Kindle books (MOBI, AZW, AZW3, PRC) | sniffed as `Format::Mobi`, then "not supported yet" |
| FictionBook (FB2, FB2.ZIP), common for Russian-made and many free libraries | sniffed, not opened |
| Comics (CBZ) | sniffed, not opened |
| DRM-protected Kindle files told apart from damaged ones | no |
| The first Indic PDF of a run | 110 to 400 ms on the phone, building reference-font tables every launch |
| Redrawing a reflowed screen | 15 ms on the phone, glyphs rasterised from outlines every time |
| Pictures on pages turned by `/Rotate`, in reflow | drawn turned |
| A hyphen at the bottom of a page | allowed |
| Page-view calls | named for PDF (`renderPdfTile`), though CBZ will need them too |

## 6.1 Corpus and truth

- **Writer:** Calibre 7.6 (Ubuntu package) converts our EPUB sources into
  MOBI (KF7), AZW3 (KF8), combined MOBI (KF7 + KF8 in one file) and FB2.
  Calibre is an independent implementation, so Akkhor reads files it did
  not write itself.
- **Sources:** the Bangla, Hindi and English sample books, extended to
  cover headings, paragraphs, lists, a table, pictures, internal links,
  footnotes, a cover and a table of contents. Free fonts only; no
  copyrighted text.
- **Truth:** the source EPUB as Akkhor already reads it (checked in Phase
  1). Per chapter, the MOBI, AZW3 and FB2 text must match it word for
  word (whitespace aside), with the same chapter breaks, headings,
  pictures and table of contents entries.
- **Other readers, scored the same way:** MuPDF (opens FB2, MOBI and CBZ),
  KindleUnpack (`mobi` on PyPI) and Calibre's own text output.
- **Comics:** CBZ files made by a script: pages as JPEG, PNG, WebP and
  GIF, odd sizes, a double-page spread, `ComicInfo.xml` with and without
  a right-to-left manga flag, junk entries (`__MACOSX/`, `Thumbs.db`),
  names that sort wrongly as text (`p2`, `p10`).
- **Hostile files:** truncated, record offsets out of range or out of
  order, wrong compression type, huge declared sizes, index loops, an
  encryption flag set, a KFX container, a RAR file named `.cbz`,
  billion-laughs-style FB2.
- **Real files:** any DRM-free MOBI, AZW3, FB2 or CBZ Misbah has, measured
  locally and never committed (numbers only, as with the PDFs).

## 6.2 PalmDB and MOBI (KF7)

PalmDB header and record list; record 0's PalmDOC header (compression:
none, PalmDOC LZ77, HUFF/CDIC), MOBI header (text encoding CP1252 or
UTF-8, first content and image records, EXTH flag, extra-data flags) and
EXTH (title 503, author 100, publisher 101, language 524 or the locale
field, cover 201 and thumbnail 202, KF8 boundary 121). Text records
decompressed one at a time, with their trailing entries (multibyte and
indexing data) removed. The HTML that results becomes a `Document`
through the existing markup reader: `<mbp:pagebreak/>` splits chapters,
`filepos` links become internal links, `recindex` pictures become images,
the NCX index gives the table of contents. Old PalmDOC books (`.pdb`,
`TEXtREAd`) read as plain text.

## 6.3 KF8 (AZW3, and the KF8 half of combined files)

FDST flows; the SKEL and FRAG indexes (INDX records with TAGX tag tables
and CNCX strings) rebuild each file's XHTML from its skeleton and
fragments; `kindle:embed:` resolves pictures, `kindle:flow:` style
sheets; embedded fonts (FONT records, zlib, XOR-obfuscated) are used like
EPUB fonts; the NCX gives the table of contents. A combined file is read
from its KF8 half.

## 6.4 DRM and what is not supported

Never opened, always named: MOBI with an encryption type set (Kindle DRM)
→ "protected"; KFX (`CONT` / `DRMION` containers), Topaz, CBR (RAR) and
CB7 (7-Zip) → "not supported" with the format's name, so the app can say
which. Encrypted fonts inside a DRM-free file are de-obfuscated as the
format defines.

## 6.5 FB2

XML with its declared encoding (UTF-8, Windows-1251, Windows-1252,
KOI8-R), as `.fb2` or zipped `.fb2.zip`. `description` gives title,
authors, language, annotation and cover; the main `body` gives chapters
(top-level sections), the notes body gives footnotes, linked both ways.
Elements: section, title, subtitle, p, emphasis, strong, strikethrough,
sub, sup, code, epigraph, cite, poem, stanza, v, text-author, empty-line,
table, image, a. Pictures are base64 `binary` elements, decoded only when
drawn. Entity expansion is not supported (FB2 does not need it), so
billion-laughs input stays flat.

## 6.6 CBZ

A zip of pictures in natural order (`p2` before `p10`), skipping folders,
`__MACOSX/` and non-images. JPEG, PNG, WebP and GIF (first frame).
`ComicInfo.xml`: title, series, number, writer, language, front cover, and
`Manga=YesAndRightToLeft` → right-to-left reading, reported in `info`. The
page view works as for PDF: tiles at any zoom from one decoded page kept
under the byte budget; a zoomed-out JPEG decodes at a reduced scale.
Themes keep the pictures' colours (85% brightness on dark themes, as for
PDF photos). No text: search ends at once, reflow is not offered.

## 6.7 Same calls for every format

- Sniffing by magic bytes (`BOOKMOBI`, `TEXtREAd`, `<FictionBook`, a zip
  of pictures, a zip holding one `.fb2`), the extension only as a tie-break.
- MOBI, AZW3 and FB2 are reflowable books: chapters, locators, progress,
  search, ranges, sentences and covers work through the calls that serve
  EPUB today, with no new app code. CBZ is a page book: page locators,
  progress by page, page view.
- **Interface clean-up, while no app exists:** page-view calls get
  neutral names for PDF and CBZ (`renderPageTile`, `renderPageTiles`);
  `format` codes for the new formats; `info` gains `direction` (`ltr` or
  `rtl`) and `series`. Native.kt, the JVM test and `docs/api/android.md`
  follow. Once the app starts, changes are additions only.

## 6.8 Engine items carried from Phase 5

- **Reference-font tables kept on disk:** the app gives a cache folder
  (`setCacheDir`); tables are stored per font file (content hash) and
  engine version, loaded in a few milliseconds. Target: the first Indic
  PDF's card under 50 ms on the phone from the second launch on.
  *Result (VM):* tables built in 2 to 85 ms read back in 0.04 to 0.8 ms
  (`data/ref_tables.csv`); phone run in 6.11.
- **Glyph bitmap cache for reflowed screens:** rasterised glyph masks per
  font, glyph, size and subpixel offset, under a byte budget. Target:
  redrawing a screen at least twice as fast as now, pixels unchanged.
  *Result:* redraw 2.1 to 2.7 times faster on all 8 test books, the
  first and next screens never slower (`data/glyph_masks.csv`). Pixels
  match outline drawing within a quarter-pixel position step: edges move
  by at most 1/8 px (largest channel change 49, mean under 0.5); at
  1/64 px steps the difference is 4, so the blending itself is exact.
- **Pictures on turned pages** drawn upright in reflow.
- **No hyphen on a page's last line.**

## 6.9 Fuzzing

New targets: `mobi` (PalmDB, both decompressors, KF7 and KF8, indexes),
`fb2`, `cbz`. Every input that crashes or hangs becomes a test first.

## 6.10 Size

Switches `mobi`, `fb2`, `cbz` (WebP decoding inside `cbz`), all on in the
app build. Measured per switch on arm64. Budget: the three together under
0.5 MB.

## 6.11 Measure and review

Corpus scores for Akkhor, MuPDF, KindleUnpack and Calibre; open, card,
chapter and page timings on the VM and on the phone (`akkhor-bench
phone` and `library`); case study, CSVs, `history.csv`; review in
`docs/reviews/phase-6.md`.

## Done when

- [x] Corpus with truth: MOBI, AZW3, combined MOBI and FB2 from Calibre; CBZ from a script; hostile files
- [x] MOBI (KF7) and PalmDOC: text, pictures, table of contents, metadata, cover, both decompressors (links: anchors kept as `#filepos`, followed in 6b.2)
- [x] KF8 (AZW3 and combined files): text, style sheets, pictures, table of contents (embedded fonts not used, as for EPUB; in the review)
- [x] DRM and unsupported containers named, never opened
- [x] FB2 and FB2.ZIP: text, notes, pictures, metadata, cover, encodings
- [x] CBZ: page view, natural order, ComicInfo (right-to-left), cover
- [x] Same calls for every format; page-view calls renamed; Native.kt, JVM test, API doc
- [x] Reference-font tables on disk; glyph bitmap cache; turned pictures upright; no hyphen on a page's last line
- [x] Fuzz targets for the new formats (3 bugs found and fixed, 2 more by review: `data/fuzz_phase6.csv`)
- [x] Size per switch measured (MOBI 56 KB, FB2 38 KB, comics 209 KB, together 306 KB on arm64)
- [x] Case study data, review (`docs/reviews/phase-6.md`), CI green
- [ ] Phone numbers for the new formats (needs a run on the phone)

## Risks

| Risk | Mitigation |
|---|---|
| KF8 indexes are intricate and lightly documented | Calibre writes them, KindleUnpack reads them: both are checks; combined files fall back to the KF7 half if the KF8 half fails |
| HUFF/CDIC compression: Calibre does not write it (old kindlegen did) | a small hand-built file for the decoder, plus any old DRM-free Kindle book Misbah has |
| Comics are big pictures (2,000 × 3,000 pixels and more) | one decoded page under the byte budget; reduced-scale JPEG decoding; never more than two pages decoded at once |
| Renaming calls breaks nothing only while no app exists | done in this phase, before the app starts |
