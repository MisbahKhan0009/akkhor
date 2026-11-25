# Real-world PDFs

Generated corpora are cleaner than real books. Real Bangla and Hindi PDFs
are measured here **only when they may be committed** (the author's own
documents, permissively licensed books), each with a hand-checked ground
truth. None is committed yet.

Copyrighted books are measured on the developer's machine with
`scripts/bench/score_local.py` and never added to the repository; only
the defects they reveal are reproduced on our own corpus files
(`scripts/make_producer_quirks.py`) so CI keeps them fixed.

## Findings so far

**A 204-page Bangla book made in Word, printed by Acrobat and rewritten by
Ghostscript** (Kalpurush body, Hind Siliguri headings; measured 2026-11-25):

| Finding | Effect before | Fix |
|---|---|---|
| Bold text drawn twice on itself (fill, then a stroke pass) | every letter doubled: `ককপিিপররাাইইটট` | a glyph drawn again within 0.03 em is one glyph, marked bold (`*.fakebold.raw.pdf`) |
| Font programs rewritten without `cmap`, `name`, `post`, with side bearings changed | outlines shifted sideways, so outline matching failed even with the right font registered | outline hashes ignore horizontal position (`*.rewritten.raw.pdf`) |
| No usable text layer for conjuncts (`ToUnicode` maps them to nothing or to private-use codes) | conjuncts dropped: `লনজনিত` for `লঙ্ঘনজনিত` | recovery needs the font: Kalpurush (OFL) and Hind Siliguri's weights (OFL) as references |

Words intact, against Akkhor's own text with Kalpurush and Hind Siliguri
registered (spot-checked on three pages against the page images):

| Engine | Words intact |
|---|---|
| Akkhor, book's fonts registered | 100% (the reference) |
| Akkhor, bundled fonts only | 68.2% |
| MuPDF 1.28.6 | 26.1% |
| PDFium 156 | 25.9% |
| Poppler 24.02 | 25.7% |

Open: reading order on card and table layouts (drawing order puts
footnotes first and labels apart from their text), which Phase 4.3's page
structure addresses.
