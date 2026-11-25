# Review: Phase 4 (Indic PDF reflow)

Date: 2026-11-25. Scope: the book corpus and scorer, word gaps, the text
layer fixes (Typst spans, fake bold, rewritten fonts, Type 3 advances and
shapes, clipping), `structure`, `reflow`, the CLI, Android and `akkhor`
interfaces, standard fonts as a switch, the benchmark scripts, as of
commit `16f0af0` plus the fixes below. Method: line-by-line reading against
the code-review checklist, hostile inputs for each new code path, every
test checked to fail without its fix, a real 204-page Bangla book measured
locally, and the engine comparison rerun on an idle machine.

Severity: 🔴 must fix (wrong output, crash) · 🟠 should fix (performance
cliffs, fragile behaviour, missed targets) · 🟡 improve or document.

## Evidence

| Check | Result |
|---|---|
| Book corpus, 22 PDFs (Chrome, Word via LibreOffice, XeLaTeX, Typst; Bangla, Hindi; one and two columns) | words intact 99.6% (page text; the rest are line-end hyphens, which reflow joins); reflow: 0.0% wrong characters, block starts 100%/100%, headings 100% at the right level, footnotes 100%, no furniture leaked (`tests/reflow.rs`) |
| Same corpus, other engines (words intact) | MuPDF 69.5%, PDFium 68.9%, Poppler 83.1%; on XeLaTeX MuPDF 5.4% |
| Real 204-page Bangla book (not committed) | Akkhor 100% with its free fonts registered, 68.2% with bundled fonts; MuPDF, PDFium, Poppler about 26% |
| Positions | every reflow word maps to the same word on its page and back (`reflow_positions_map_back_to_pages`, Android test) |
| Structure and reflow time, page text built | 0.65 ms worst 16-page chapter (2,000-page PDF), 0.42 ms (real book) |
| Hostile page: 20,000 side-by-side items | **stack overflow (abort)** before the fix (finding 1); 0.76 s after |
| Damaged files (200, same rule for all engines) | Akkhor shows a page for 143, MuPDF 130, Poppler 117, PDFium 109; no crash or hang |
| `libakkhor.so` arm64, all formats (CI) | 5.81 → 6.01 MB with reflow and Type 3, **5.73 MB** with standard fonts from the app; target 5.5 MB not met (finding 12) |
| CI | **red for three pushes** on a lint only CI's newer clippy has (finding 11) |

## Findings

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 1 | `structure::cut` | Column cuts recursed once per column. A page with thousands of side-by-side items (broken or hostile) overflowed the stack: an abort that `catch_unwind` cannot catch, so the app would crash. | 🔴 | Fixed: cuts stop at depth 24; test with 20,000 items. |
| 2 | glyph collection | Real book: bold drawn as a fill and then a stroke pass doubled every letter (ককপিিপররাাইইটট). | 🔴 | Fixed: a glyph drawn again within 0.03 em (same font, glyph and code, page-wide grid) is one glyph, marked bold. Reproduced on corpus files; test fails without the fix. |
| 3 | `read::word_pieces` | Typst closes `/ActualText` spans mid-cluster and maps glyphs to whole clusters: letters repeated (মানুুষ, পুপুকুরঘাটে, কোইই). | 🔴 | Fixed: marks, rephs and phalas after a span, split spans with the same text and bases drawn before their span join it. Typst words intact 85% → 100%, also with no fonts registered (`books_no_fonts.rs`). |
| 4 | glyph collection | Type 3 advances were multiplied by 1000 again (hayro-font already scales them): heading boxes thousands of points wide, so selection and word gaps were wrong on every Chrome synthetic-bold heading. | 🔴 | Fixed. |
| 5 | `reference::outline_hashes` | Ghostscript rewrites embedded TrueType programs; outlines shift sideways and exact matching found 61 of 127 glyphs of the right font. | 🟠 | Fixed: hashes take x from the first point. 127 of 127. Test fails without the fix. |
| 6 | glyph collection | Chrome draws the first glyphs of the next page's lines and clips them away; they were read (`ती है आँ बै` at a page's foot). | 🟠 | Fixed: glyphs whose middle is outside the clip are dropped. |
| 7 | `text::assemble` | XeLaTeX draws no spaces and squeezes justified gaps under the fixed 0.22 em threshold: words ran together (89.4% intact). | 🟠 | Fixed: threshold from the line's own gaps. 100%. |
| 8 | flow layout | A heading could end a page with its text on the next (seen on the reflow showcase); also in EPUBs. | 🟠 | Fixed: keep-with-next for two lines; test fails without it. |
| 9 | Type 3 recognition | Each page recognised its shapes again against every registered font, and every glyph's grown shape was stored: with 40 phone fonts, seconds and ~10 MB on first use. | 🟠 | Fixed: answers cached across pages, at most 24 fonts pictured, grown shapes computed on the fly (264 bytes a glyph). First use about 100 ms with 6 fonts. |
| 10 | benchmarks | Two methodology errors in my own comparison: "opened" counted zero-page opens (MuPDF 180 looked far ahead; with one rule Akkhor leads 143 to 130), and MuPDF and PDFium rendered three bytes a pixel against Akkhor's four (MuPDF's memory looked 2 MB lower). | 🟠 | Fixed and republished with the corrections stated in the case study. |
| 11 | process | CI's stable clippy (1.100) is newer than the local one (1.97, which cannot be updated here); the same lint (`chunks_exact_to_as_chunks`) broke CI twice, and three pushes went red before I looked. | 🟡 | Fixed the code. Rule: check CI after every push before reporting; the lint is noted for local review. |
| 12 | size | arm64 5.73 MB, target 5.5 MB (PDF only 5.02 MB, target 4.5 MB). Reflow, structure and Type 3 recognition added 0.2 MB; standard fonts from the app saved 0.27 MB. The host-build measurement first suggested +1.8 MB: on x86_64 PDF rendering was left out of the library (bitmap locking is Android-only) until reflow pictures made it reachable. | 🟠 | Target missed again by 0.23 MB; carried. CI arm64 is the only size measure from now on. |
| 13 | text | Mukta in XeLaTeX: nukta and candrabindu glyphs read wrong in two words (99.5%). | 🟡 | Open. |
| 14 | text | A font the reader does not have (Hind, Mukta not registered) still leaves XeLaTeX PDFs with wrong conjuncts, and their reflow follows (CER 14%). | 🟡 | Documented: the app should offer the free Bangla and Hindi fonts (Kalpurush, Hind Siliguri weights, Mukta); this is what takes a real book from 68% to 100%. |
| 15 | structure | Tables and card layouts (the real book): cells are read in column order, centred cell lines split paragraphs, a bold line ending in a colon becomes a heading. | 🟡 | Open: table detection is later work; reflow stays optional in the reader. |
| 16 | redraw | Study: `vello_cpu` 0.3 has no way to record a page and replay it, and hayro interprets the content stream on every draw. Redraw stays at 2.3 ms against MuPDF's 1.6 ms. | 🟡 | Closed as a study: revisit when vello_cpu can record; the app keeps drawn tiles. |
| 17 | memory | Indic page at 2x: 11.6 MB against 10.7 (MuPDF) with the same pixel format; the difference is a constant ~4 MB above the pixels at 1x, 2x and 4x (code paged in, parsed fonts), not growth. | 🟡 | Documented; sharing parsed fonts between text and drawing not worth it now. |
| 18 | damaged files | 2 of Akkhor's 57 refusals are files MuPDF shows (a damaged page dictionary it repairs); the other 55 have no readable structure for either. | 🟡 | Open, low value. |
| 19 | structure | On a one-page document, a small line in the top or bottom 8% is taken for a running header (there is no other page to compare with). | 🟡 | Documented. |
| 20 | page text | Page text keeps line-end hyphens, so searching the page view for a hyphenated word misses it (reflow joins them). | 🟡 | Open: search could join them as reflow does. |
| 21 | corpus | LuaLaTeX could not be added (no `luaotfload` here); no permitted real PDF is committed yet. | 🟡 | Open; the real book is measured locally and its defects reproduced on our files. |

## What holds up

- Reflow is close to exact on every producer, with positions that map
  both ways, and it costs under a millisecond per chapter on top of the
  page text.
- Every fix found on the real book is reproduced on a file we can commit,
  and its test fails without the fix.
- The comparison is fairer than at the end of Phase 3: the same "opened"
  rule and pixel format for every engine, and two of my own earlier claims
  were withdrawn because of it.

## Result

🔴 four found, all fixed. 🟠 fixed except the size target (5.73 of
5.5 MB), carried. 🟡 fixed or documented; tables, Mukta's two glyphs and
hyphen-aware page search remain open.
