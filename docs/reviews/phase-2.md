# Review: Phase 2 (Indic text in PDFs)

Date: 2026-11-25. Scope: `akkhor-pdf` (`glyphs`, `recover`, `reference`,
`read`, `text`), `akkhor-flow::bijoy`, the CLI and Android additions, the
corpus and CI, as of commit `491d239` plus the fixes below. Method:
line-by-line reading against the code-review checklist (correctness,
performance, robustness, maintainability), and new evidence: a corpus with
a second font design per script, glyph names switched off, unknown fonts,
38 registered reference fonts, the older `indic.pdf` fixture, and a CI run
without the `bundled-fonts` feature.

Severity: 🔴 must fix (wrong output, crash) · 🟠 should fix (performance
cliffs, fragile behaviour) · 🟡 improve or document.

## Evidence

| Check | Result |
|---|---|
| 36 corpus PDFs (Chrome, LibreOffice; Noto + Hind Siliguri / Mukta; Bijoy; with and without ToUnicode) | 0.0% character errors on all, with and without glyph names; read glyph by glyph through `ToUnicode` alone, 2 to 100% wrong (see the correction below) |
| Same designs, font not registered ("unknown font") | Chrome: 27% → 14% (Bangla), 20% → 19% (Hindi); LibreOffice: 45% → 45%, 42% → 42% |
| Bijoy round trip through `poriborton` | 99.7% of 3,842 words exact; all 11 misses are words `poriborton` cannot encode |
| First page of text, tables not yet built (desktop core) | 25 to 50 ms; again: about 1 ms |
| Same, 38 reference fonts registered (26 MB) | 24 to 50 ms after fix 2 (Mukta PDF was 206 ms) |
| `indic.pdf` fixture (Chrome, bold headings) | body text exact; **bold headings lose letters** (finding 6) |
| CI without `bundled-fonts` | **Bijoy PDF without ToUnicode failed** (finding 4) |
| `libakkhor.so`, arm64 | 6.27 MB (Phase 1: 5.8 MB); target 3 MB stays a Phase 3 item |

## Findings

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 1 | `is_bijoy_font` | "SutonnyOMJ" and every name starting "Sutonny" were treated as Bijoy, but the OMJ fonts are the Unicode versions: their text would have been converted a second time into nonsense. | 🔴 | Fixed: OMJ excluded; and a run that already contains Bangla letters is never converted, whatever the font is called. Tests for both. |
| 2 | `reference::recover` | Matching scanned every registered reference font and built each one's outline index on first use. With the phone's fonts registered, the first text request would index all of them. Separately, a font without `uniXXXX` glyph names (Mukta) always needed the deep build: 206 ms for one page, about 0.6 s on a phone. | 🟠 | Fixed: a reference with the same PostScript name is tried first; the font's own `GSUB` tables now suggest which clusters to shape, verified by shaping (deep build no longer needed for any corpus font). |
| 3 | `recover` | Candidates the shaper finds malformed (it inserts a dotted circle) were still learned from, and guessed clusters spelled with Assamese ৰ were recorded as glyph sequences that would have turned Bangla র্ব্য into ৰ্ব্য. | 🔴 | Fixed: no learning from dotted-circle output; hints with ৰ ৱ dropped (they share glyphs with র ব). |
| 4 | tests, CI | The accuracy tests relied on `bundled-fonts` for the Latin reference font; CI runs without it and the LibreOffice Bijoy PDF without ToUnicode failed (91%). The test, not the engine, was wrong, but it shows the engine's quality depends on which fonts are registered. | 🟠 | Fixed: the tests register every font they rely on. |
| 5 | `page_text` | Any number of embedded fonts was recovered per page; a hostile file with thousands of fonts would hash and match each. | 🟠 | Fixed: at most 48 fonts per page are recovered, the rest use their text layer. |
| 6 | glyph collection | Chrome draws synthetic bold (bold text without a bold font) as **Type 3 fonts**: glyph procedures, no font program, no ToUnicode. Recovery has nothing to match, so bold Indic headings lose letters (`অক্ষর পরীক্ষ` for `অক্ষর পরীক্ষা`). | 🟠 | Open, planned: hash the Type 3 glyph paths against reference outlines (scaled by the font matrix). Added to Phase 4. |
| 7 | design | Fonts the engine does not know (not bundled, not registered) cannot be recovered; LibreOffice PDFs then stay at their text layer's 42 to 45% error. Bangla books mostly use SolaimanLipi, Kalpurush, Nikosh, Siyam Rupali; none is a reference today. | 🟠 | Documented. A test keeps unknown fonts no worse than the text layer. The app must register the phone's Indic fonts (`registerFont` now does); a downloadable pack of freely licensed Bangla fonts is planned. |
| 8 | corpus | Every test PDF is generated (two producers, our own texts). No PDF from Word, InDesign, XeLaTeX or a real publisher has been measured; those are the PDFs users have. | 🟠 | Open: collect real Bangla and Hindi PDFs with permission and add their measured error rates to CI. |
| 9 | `page_text` | 100 lines of nested closures in one function: hard to read and to test. Sequence matching allocated a vector per glyph. | 🟡 | Fixed: `read.rs` (`readings`, `word_pieces`, `bijoy_piece`), one id slice per word. |
| 10 | Android | `pdfSelectionRects` with an end before the start used the end offset of another page. | 🟡 | Fixed: empty selection. |
| 11 | `recover` | Noto names its nukta + virama mark `uni0921093C094D` ("ड़्"); alone it reads as ड़्. Glyph sequences repair the common clusters, a nukta conjunct the candidates never formed would read wrong. | 🟡 | Documented; rare (nukta conjuncts). |
| 12 | Bijoy in PDFs | A Bijoy word is one span: selection and highlights are per word, not per letter. A comma inside a Bijoy word is read as ‚ (ূ-kar) because many fonts share the outline. | 🟡 | Documented. |
| 13 | Android | Positions are UTF-8 byte offsets. Copy and search work, but read-aloud that highlights words in a Kotlin string will need UTF-16 offsets. | 🟡 | Open: add when the reader app needs it. |
| 14 | plan | Phase 2.4 promised two-column detection; the engine reads in drawing order, which is reading order for every producer tested. | 🟡 | Plan corrected; column detection moves to Phase 4 (reflow needs it anyway). |
| 15 | plan | Kruti Dev was planned as experimental; there is no open table or corpus to check one against. | 🟡 | Moved to a later phase (done in 2.5). |
| 16 | `recovery` | The per-document recovery cache is locked while a table is built (tens of ms): a second thread asking for text waits. Rendering is not affected. | 🟡 | Documented. |

## What holds up

- The revised design is general: nothing is specific to one font or
  producer. Two independent designs per script and both producers reach
  0.0%, also with glyph names ignored, which is how many phone fonts ship.
- The tables are built lazily and once per process; rendering never pays
  for them.
- Every recovery and extraction step runs inside `catch_unwind`; the
  robustness test now also drives the text layer, search and selection on
  damaged files, including LibreOffice output.

## Result

Phase 2's done criteria are met on the corpus. Findings 1 to 5, 9 and 10
are fixed; 6, 7, 8 and 13 are the real-world gaps and are carried into the
plan.

## Correction (added in Phase 3)

The "before" numbers in this review read each glyph's `ToUnicode` entry,
as `hayro` exposes it. Comparing with MuPDF, PDFium and Poppler in Phase 3
showed that Chrome and LibreOffice also write `/ActualText` spans with the
correct text of every cluster, which those engines read: they reach under
1% on the "as produced" corpus. So finding 7 (unknown fonts stay at
42 to 45%) was a gap in Akkhor, not in the PDFs. Akkhor reads
`/ActualText` since Phase 3, and the corpus's "no ToUnicode" PDFs no
longer keep those spans, so they test recovery from glyphs alone. Data:
`docs/case-study/data/text_accuracy.csv`.
