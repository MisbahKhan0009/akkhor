# Table and card corpus

Bangla and Hindi pages with tables and cards, made by
`scripts/make_tables.py` from `bn-tables.txt` and `hi-tables.txt`, through
four producers: Chrome (HTML), LibreOffice Writer (a Word .docx), XeLaTeX
(booktabs, tcolorbox) and Typst. Each PDF has a truth beside it: `.json`
(blocks in reading order; a table's rows and whether it should reflow as
text or as a picture; a card's texts) and `.expect` (the same truth as the
reflow should give it, one block per line, for the Rust tests).

What it covers:

| | Chrome | Writer | XeLaTeX | Typst |
|---|---|---|---|---|
| Two-column word list | ruled (cell borders) | ruled ("Table Grid") | ruled (`\|l\|l\|`, `\hline`) | ruled (default strokes) |
| Three columns, last one wrapping | no rules | ruled, split over two pages | booktabs (three rules) | no rules |
| Six columns of marks (picture) | ruled | ruled | booktabs | row strokes only |
| Full-width card, centred lines | shaded `div` | shaded table cell | `tcolorbox` | filled `block` |
| Three cards side by side | CSS grid | one-row table, shaded cells | `tcbraster` | `grid` of blocks |
| Bold lead-in ending in a colon | yes | yes | yes (no bold font: drawn regular) | yes |

Fonts: Noto Serif Bengali and Noto Serif Devanagari (regular only, so
Chrome draws bold as Type 3 glyphs and XeLaTeX and Typst draw it regular).
Scored by `tables_and_cards_reflow_like_the_source` in
`crates/akkhor-pdf/tests/reflow.rs`.
