# Review: Phase 4b (tables, cards and the open items)

Date: 2026-11-26. Scope: `akkhor_pdf::tables`, the table, box, centring,
lead-in and label rules in `structure`, the table and box output of
`reflow`, line splitting at cell gaps and vertical rules in `text`, Type 3
bold by ink, the Typst reph fix, hyphen-aware page search, Mukta
candidates, the vendored parser change, reflow context pages, the table
corpus and its scorer, as of commit `2f671d1` plus the fixes below. Method:
line-by-line reading against the code-review checklist (correctness, edge
cases, cost, hostile input), every new rule checked to fail its test
without it, the real 204-page Bangla book measured locally before and after
(Phase 4 build against this one) and spot-checked on page images, and the
other engines scored on the same table corpus.

Severity: 🔴 must fix (wrong output, crash) · 🟠 should fix (performance
cliffs, fragile behaviour, missed targets) · 🟡 improve or document.

## Evidence

| Check | Result |
|---|---|
| Table corpus, 8 PDFs (Chrome, Word via LibreOffice, XeLaTeX, Typst; Bangla, Hindi) | every block matches the truth (`tables_and_cards_reflow_like_the_source`): 0.0% wrong characters, text rows and pictures where the truth says, cards whole, 5 of 5 headings and no extra |
| Same corpus, other engines (their page text) | table rows in order: MuPDF 65%, PDFium 76%, Poppler 48%; cards whole: 69%, 75%, 28%; words intact: 77%, 72%, 85% (Akkhor 100% on all three) |
| Book corpus, 22 PDFs | unchanged: words intact 99.6%, reflow 0.0% wrong characters (footnote markers aside), block starts 100% |
| Real Bangla book (local) | Phase 4: 1,221 headings, tables read down their columns. Now: 356 headings, 585 rows on 53 pages read across; 2 table pages checked against images |
| Page search | 9 corpus files missed a word split by a line-end hyphen; none now |
| Damaged files | 145 of 200 show page 1 (was 143); renders of every corpus PDF identical to the previous build |
| Cost | page text of the 2,000-page PDF 4.06 s against 4.13 to 4.38 s for the previous build, same session; structure and reflow 0.75 ms worst chapter; the pictures, rules and boxes pass 0.4 ms a page |
| Hostile pages | 10,000 rules and 20,000 aligned lines on one page: before the fix 3.6 s in `tables::find` (finding 3), now under 0.1 s |

## Findings

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 1 | `text::split_at_gutters` | First version of the cell rule tested the middle of the shared gap: a short cell whose text reached into it (header "বিজ্ঞান" over narrow numbers) counted as crossing, and whole numeric columns stayed merged. | 🔴 | Fixed: the rule now asks for free space anywhere in the open part. |
| 2 | `structure::paragraphs` | The usual line step was the region's 25th percentile; on a page of headings, short paragraphs and a table it was a paragraph gap, so a lead-in merged with the paragraph above (XeLaTeX, Typst). Cards of two lines had no step of their own at all. | 🔴 | Fixed: smallest step per text size over the page (never a paragraph gap). Book corpus unchanged. |
| 3 | `tables::find` | Lines were assigned to grid cells by testing every line against every cell: rows × columns × lines, 3.6 s on a hostile page. Rule pairs, gaps and the unruled scan were also unbounded. | 🟠 | Fixed: binary search per line; caps of 3,000 gaps and 1,500 rules a direction; unruled runs bounded to 300 bands. Test `thousands_of_rules_and_aligned_lines_stay_quick`. |
| 4 | `structure::analyse` | A table row cut off by a page break was read as three cards: furniture is not marked when the next page is laid out, so "the last block of the page before" was its page number. | 🔴 | Fixed: the bottom band is skipped. Test: `hi.writer.pdf`. |
| 5 | numbering columns | First version folded any all-number column, which turned the six-column marks table into text rows. | 🔴 | Fixed: only a first column counting up by one. |
| 6 | real book | Coloured labels and row-number squares (tiny fills, framed squares) became 956 "cards"; with them gone, the labels became headings (829). | 🔴 | Fixed: boxes need two lines' height and four ems' width; three or more same-size candidates in a row are labels; lead-ins and numbers are never headings; text over a picture belongs to it. |
| 7 | `read::word_pieces` | Typst draws a reph cluster's base before its span; with a full reference font registered the base read "ড" and the span "র্ড", so কার্ড came out কাডর্ড. Older than this phase, found by the new corpus. | 🔴 | Fixed: the reph-first spelling is matched. |
| 8 | copyright | A first draft of the case study quoted a table row of the real book; the tool refused the write. Only counts are published; `real_world_tables.csv` names no title. | 🟠 | Fixed before commit. |
| 9 | test design | Cards are scored whole, not paragraph by paragraph: with no bold font (XeLaTeX, Typst) and no paragraph space inside the box, a card's title and body cannot be told apart on the page. I changed the expectation after the first run; the reason is in `make_tables.py`. | 🟡 | Documented. |
| 10 | corpus | The table corpus is mine, made in the same phase as the rules: they can fit it. The real book is the only independent check, and only by counts and two pages. | 🟠 | Open: a permitted real PDF with tables in `tests/real/`. |
| 11 | real book | A header row whose labels sit in coloured boxes is one line above the table, not its first row. | 🟡 | Open. |
| 12 | bold | Chrome's Type 3 bold is now seen (ink of 8 probed glyphs against the reference font); XeLaTeX and Typst with a font that has no bold draw regular text, so a header row there is plain. | 🟡 | Documented. |
| 13 | headings | A bold short line is a heading only when body text follows (agreed in the plan); "Exercise" followed straight by a table is now a paragraph. | 🟡 | Documented; revisit with real examples. |
| 14 | search | Flow search also finds text in an image block's alt text (wide tables, picture labels): the hit jumps to the picture, with no highlight inside it. | 🟡 | Documented. |
| 15 | Mukta | Mukta draws ब्ज़ and ब्ज with one glyph: सब्ज़ियाँ comes out without its nukta, and nothing in the PDF says otherwise (99.8% intact). | 🟡 | Closed: not recoverable without a dictionary. |
| 16 | parser | The vendored `hayro-syntax` now skips an unparsable array element. Only content-stream-free arrays are affected, and renders of every corpus PDF are unchanged, but it is a second patch to keep when updating. | 🟡 | Documented in `vendor/README.md`, diff kept. |
| 17 | reflow | One-page chapters (common with outlines) had no neighbours to tell running headers by. | 🟠 | Fixed: the pages around a chapter are read as context. |
| 18 | text | Unrelated glitch seen on the real book: one word with a pre-base vowel sign before a three-letter conjunct reads in the wrong order. | 🟡 | Open (text layer). |

## What holds up

- Every table and card in the corpus, from four producers and three kinds
  of table, reflows exactly, and the real book's tables read across where
  they read down before.
- The book corpus did not move: nothing in the new rules was bought with a
  regression there.
- Each 🔴 above has a test that fails without its fix.

## Result

🔴 six found, all fixed. 🟠 fixed except an independent real PDF with
tables (carried). 🟡 documented or carried: boxed header labels, headings
before tables, alt-text search hits, one text-layer glitch.
