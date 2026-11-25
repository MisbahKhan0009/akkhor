# Phase 4b: tables, cards and the small open items

Status: in progress. Follows [phase-4.md](phase-4.md) and its review
([reviews/phase-4.md](../reviews/phase-4.md)); the library size (finding 12)
is accepted as it is.

## Starting point (2026-11-25)

| Open item | What happens now |
|---|---|
| Tables (review finding 15) | No table detection. The column cut finds the gaps between cells and reads a table column by column (every word, then every meaning). A ruled grid is invisible to structure: only text and pictures are looked at. |
| Cards and boxes (15) | A shaded or framed box is not seen. Two boxes side by side are read as two columns; that is right only by luck. A box's centred lines are taken for indented first lines, so each starts a paragraph. |
| Bold lead-in lines (15) | A short bold line ending in a colon ("উদাহরণ:") becomes a heading. |
| Mukta (13) | XeLaTeX with Mukta: सँभालने reads सॅंभालने (candrabindu learned as candra E plus anusvara), सब्ज़ियाँ reads सब्जियाँ (no nukta conjunct candidates). 99.5% words intact. |
| Page search (20) | A word split by a line-end hyphen is not found in the page view. |
| Damaged files (18) | 2 of the 200 damaged files open in MuPDF and not in Akkhor. |
| One-page furniture (19) | On a one-page document any small line in the top or bottom 8% is taken for a running header. |

## 4b.1 Corpus: tables and cards with ground truth

`scripts/make_tables.py` builds Bangla and Hindi pages from four producers
(Chrome, Word via LibreOffice, XeLaTeX, Typst), each with a truth JSON:

- a two-column word list (word, meaning) with a header row, ruled;
- a three-column table (word, meaning, example sentence) whose last
  column wraps onto several lines;
- a five-column table of numbers (marks by subject), wider than a phone;
- an unruled table (XeLaTeX `booktabs`: only three horizontal rules; and
  a Chrome table with no borders);
- a full-width shaded card with a bold title and centred body lines;
- two or three cards side by side (Chrome grid, Typst grid, XeLaTeX
  minipages; Writer has no side-by-side cards that are not a table);
- a bold lead-in line ending in a colon, followed by a list or a table;
- ordinary paragraphs and headings around them, so nothing else regresses.

Truth kinds: `heading`, `paragraph`, `table` (rows of cells, and whether
it should reflow as text or as a picture), `card` (its texts in order).

## 4b.2 Rules and boxes from the page

The picture pass (`theme::picture_boxes`) also collects vector shapes:

- **rules**: horizontal or vertical segments under 2 pt thick, stroked
  lines and thin filled rectangles alike, merged when they touch;
- **boxes**: filled rectangles in a colour other than the page's, and
  stroked closed rectangles (rounded corners allowed), big enough to hold
  a line of text, not covering most of the page.

## 4b.3 Tables

Found before the column cut, so their cells never become columns:

- **ruled**: horizontal and vertical rules that cross into a grid of at
  least 2 by 2 cells; the cells are the grid's rectangles. A 1 by 1 grid
  is a box, not a table.
- **horizontal rules only** (booktabs, many Word styles): three or more
  rules of the same span; columns from gaps that every row between them
  leaves open.
- **unruled**: at least three rows whose cells line up in three or more
  columns of short text, or in two columns where the first is narrow
  (under 35% of the width) with every row on shared baselines. Two equal
  columns of running text stay columns.

A table becomes a `Table` block: rows of cells, each cell its lines.
Reflow, the "mix" decided with the user:

- **text** when it has at most three columns: one paragraph per row, cells
  joined with ` · `, the header row bold. Positions map back cell by cell.
- **picture** when wider, or when the grid is irregular (merged cells that
  do not line up): an image block of the table's area, with all its text
  as the picture's alt text for read-aloud.

## 4b.4 Cards and boxes

A box is cut as one item of its page (like a picture), and its contents
are read on their own, so text never runs from one box into the next.
Inside a box a bold or short line is not a heading (only text at least 1.5
times the body size is), and paragraphs follow the box's own alignment.

## 4b.5 Paragraph and heading rules

- **Centred text**: when a region's lines share their centre and not their
  left edge, line length and indent no longer start paragraphs; only gaps
  and style changes do.
- **Bold lead-ins**: a bold short line ending in a colon is a paragraph
  (bold), not a heading. Other bold short lines are headings only when
  followed by body text.

## 4b.6 Mukta

Shape consonant plus candrabindu or anusvara before the vowel sign
combinations, so a candrabindu glyph is learned as ँ; shape conjuncts with
the nukta letters (क़ ख़ ग़ ज़ ड़ ढ़ फ़ य़) on either side. Target: 100%.

## 4b.7 Hyphen-aware page search

`PageText::find` also searches the text with line-end hyphens joined (the
same rule as reflow), mapping matches back so the highlight covers both
halves on both lines. Unit test plus a check on the book corpus: every
word of the truth is found by search.

## 4b.8 Smaller items

- Damaged files: look at the 2 files MuPDF opens; repair what is a common
  kind of damage, leave the rest documented.
- One-page furniture: also require a gap from the body text of at least
  two lines, and a short line.

## 4b.9 Measure, case study, review

`score_books.py` and `tests/reflow.rs` score tables and cards (rows as
text or a picture where the truth says, cards contiguous, no cell text
leaked into paragraphs). The other engines get the same table corpus for
words found. Case study, CSVs and figures updated; review in
`docs/reviews/phase-4b.md`.

## Done when

- [ ] Table and card corpus, four producers, Bangla and Hindi, with truth
- [ ] Rules and boxes collected; tables found (ruled, rules only, unruled)
- [ ] Tables reflow as text (up to 3 columns) or a picture; positions map
- [ ] Cards read box by box; centred lines and bold lead-ins handled
- [ ] Book corpus unchanged (0.0% wrong characters, 100% block starts)
- [ ] Mukta 100%; hyphen-aware page search; one-page furniture rule
- [ ] Damaged files looked at, fixed or documented
- [ ] Case study, CSVs, review; CI green

## Risks

| Risk | Mitigation |
|---|---|
| Unruled table detection taking two-column books for tables | the book corpus stays in the test; unruled needs three columns or a narrow first column |
| Picture tables lose text search in reflow | alt text kept; page view search still finds it |
| More shape collection slows reflow | measured per chapter; the shapes come from the pass that already finds pictures |
