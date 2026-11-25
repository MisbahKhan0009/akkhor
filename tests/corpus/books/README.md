# Book-layout corpus

Source texts (`bn-book.txt`, `hi-book.txt`) written for Akkhor, printed by
`scripts/make_books.py` in book layouts. Each `pdf/*.pdf` has a ground-truth
`pdf/*.json` beside it: its blocks in reading order (heading with level,
paragraph, footnote, image) and the furniture (running header, page
numbers) a reflow must leave out.

| Name part | Meaning |
|---|---|
| `bn`, `hi` | Bangla, Hindi |
| `chrome` | Chrome, HTML with CSS columns and `@page` margin boxes (no footnotes) |
| `writer` | a Word document (`.docx`) printed by LibreOffice Writer (no footnotes) |
| `xelatex` | XeLaTeX with fontspec and polyglossia, fancyhdr |
| `typst` | Typst (tags headers and footers as `/Artifact`) |
| `one` | one column, ragged right, header and page number |
| `two` | two columns, justified and hyphenated, header, page number, a figure and (XeLaTeX, Typst) a footnote |
| `hind`, `mukta` | second font design (Hind Siliguri, Mukta); otherwise Noto Serif |

LuaLaTeX is in the script but not in the corpus: the TeX installation used
to build it lacks `luaotfload`.

Scored by `scripts/bench/score_books.py` (all engines).
