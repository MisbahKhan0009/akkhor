# Phase 2: Indic text in PDFs (detailed plan)

Goal: copy, search, select and read-aloud work on Bangla and Hindi PDFs,
including PDFs whose text layer is broken and PDFs typed in legacy
encodings (Bijoy, Kruti Dev). This is the headline novelty of Akkhor.

## The problem, measured

A PDF stores glyphs, not letters. Its optional `ToUnicode` map says which
letters each glyph stands for, and for Indic scripts producers routinely get
it wrong. A one-page Bangla/Hindi PDF printed by Chrome
(`tests/fixtures/indic.pdf`), read through the current interpreter:

| Printed | Text layer says |
|---|---|
| ক্ষমা, মন্ত্রী, কর্ম | `ক্ষম , ম  , কম` (া and conjuncts lost, reph lost) |
| বিদ্যুৎ, চিহ্ন | `িবদ  ৎ`, `িচহ্ন` (pre-base vowel sign before the consonant) |
| আমাদের | `আমােদর` (split vowel ো broken in two) |
| क्षत्रिय, श्रीमान | `क्ष  य`, ` ीमान` |

Conjunct glyphs map to nothing (`\0`), vowel signs drawn before a consonant
come out first, and the reph comes out after its syllable or not at all.
The embedded fonts, however, keep their character map (`cmap`) and their
OpenType rules (`GSUB`). That is what makes recovery possible.

## 2.1 Text layer extraction

`akkhor-pdf::text`: a drawing device that, instead of painting, records each
glyph: its box on the page, `ToUnicode` text, character code, glyph id, the
font (name, a key, its program bytes) and size. `PdfDocument::glyphs(page)`.

## 2.2 Recovering letters from glyphs ("reverse shaping")

**Revised after 2.1.** Measuring the corpus showed the original assumption
was wrong: both producers drop `GSUB` from the fonts they embed. Chrome
keeps a `cmap` for the plain letters only (51 of 687 glyphs) and the
original glyph ids; LibreOffice renumbers the glyphs and writes a symbol
`cmap` from its own codes (glyph 109 is "h"). Reverse shaping on the
embedded font alone recovers almost nothing. The plan became:

1. **Reference fonts.** A glyph's outline survives subsetting. Each embedded
   glyph is hashed by its outline (skrifa, unscaled, so renumbered
   composites still match) and looked up in complete copies of fonts the
   engine knows: the bundled Noto fonts and any the app registers
   (`register_reference_font`, e.g. the phone's own Noto fonts). The
   reference with most matches wins if it matches at least half the
   subset. An embedded font that does keep `GSUB` is used directly.
2. **Reverse shaping the reference** (once per process, lazily): reverse
   `cmap`; candidate syllables shaped with HarfRust, reph first (its glyph
   is reused inside ई), then consonant + vowel sign (+ candrabindu /
   anusvara), independent vowels, consonant pairs, conjunct + vowel sign,
   reph + conjunct, and three-consonant clusters ending in a phala. A
   *deep* build adds every three-consonant cluster and only runs when a PDF
   uses a glyph the quick build missed.
3. **Sequences.** When a candidate's glyphs do not add up to its letters
   (ई drawn as इ + the reph glyph), the glyph run itself is recorded and
   matched first, longest run wins.
4. **Fallbacks, marked weak:** glyph names (`uni09470902`) and the `GSUB`
   tables read directly (a ligature means its components' letters). Weak
   entries also trigger the deep build.
5. Glyphs that draw nothing and map no character (Noto's `NullMark`, left
   where ं fused into ि) are known as empty from the start, so the fused
   glyph gets all the letters.

Choosing the text for a glyph: for an Indic font, the recovered text; else
a usable `ToUnicode` entry; else the recovered text; else nothing. A subset
`cmap` that disagrees with the reference on most glyphs is discarded.

Budget: under 50 ms per font (measured 20 to 40 ms quick, 130 ms deep on a
desktop core), once per process.

## 2.3 Visual to logical order

Glyphs come in drawing order. Per line, per script:

- a pre-base vowel sign (Bangla ি ে ৈ, Devanagari ि, and the Gurmukhi,
  Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam equivalents) moves after
  the consonant cluster that follows it;
- two-part vowels are rejoined (ে … া → ো, ে … ৗ → ৌ);
- a reph drawn at the end of its syllable moves to the start, also when it
  is fused with a vowel sign (Noto's ী + reph, Devanagari ि + reph over
  र्ति, which moves both ways at once);
- marks drawn before the rest of their cluster (a vowel sign or candrabindu
  before a ya-phala, candrabindu before the vowel sign) are swapped back;
- Devanagari ा + े/ै/ॅ/ॆ are rejoined as ो ौ ॉ ॊ;
- reordering never crosses a word gap, which is decided in drawing order
  (marks positioned back over their base do not open a gap).

## 2.4 Lines, words, reading order

Glyphs are grouped into lines by baseline (tolerance relative to font size),
sorted left to right, spaces inserted where gaps exceed a fraction of the
font size, blocks split at large vertical gaps, and simple two-column pages
read column by column. Output: `PageText { text, char boxes, lines }`, where
every character of `text` points back to a box on the page.

## 2.5 Legacy encodings

- **Bijoy** (SutonnyMJ and other "MJ" fonts), done. `akkhor_flow::bijoy`:
  longest-match pieces from a table generated by `scripts/gen_bijoy.py`
  from the MIT licensed `poriborton` crate (OpenBangla), inverted, plus its
  vowel-sign rules, plus half and below-base forms derived from its
  conjunct keys (¯ স্, ª ্র, ...); then reordering (ি ে ৈ after the
  cluster, reph © to its start, ে…া → ো, র + ্য keeps a ZWJ). Applied to
  `.txt` files detected as Bijoy, and in PDFs to every run in a font whose
  name marks it as Bijoy. Inside a Bijoy word a comma is read as ‚ (ূ-kar):
  many fonts draw both with one outline.
- **Kruti Dev / Chanakya** (Hindi): **moved to a later phase.** There is no
  openly licensed table or real corpus to check one against, and a guessed
  table would produce confident wrong text. It comes back when a corpus is
  available.

## 2.6 Search, selection and copy on PDFs

`PdfDocument::page_text(i)`, a lazy whole-document search (page by page,
stops when the caller stops), hit-testing and selection rectangles from
character boxes, all reusing the Phase 1 folding and syllable rules.
Exposed in the CLI (`--text`, `--search` on PDFs) and the Android bridge.

## 2.7 Accuracy corpus and measurement

Ground-truth texts (written for Akkhor, plus a generated "conjunct coverage"
list of every consonant pair the font joins) are printed to PDF by several
producers, and each PDF is also re-written with its `ToUnicode` maps
removed, to test recovery on its own:

| Set | Producer | Text layer |
|---|---|---|
| A | Chrome (Skia) | as produced |
| B | LibreOffice | as produced |
| C | A and B | `ToUnicode` removed |
| D | Bijoy text in a font named SutonnyMJ | ASCII |

Each of A to C is printed in two designs per script: Noto Serif (bundled)
and a second family from `tests/fonts` (Hind Siliguri, Mukta), whose glyph
names are not `uniXXXX`. The whole corpus also runs with glyph names
ignored (`accuracy_no_names`), since phone fonts often ship without them.

Metric: character error rate (edit distance / length) after normalisation.
Bijoy conversion is also checked by round trip: Unicode → Bijoy (with
`poriborton`, test-only dependency) → Akkhor → Unicode.

## Done when

- [x] Sets A and B: character error rate under 2% for Bangla and Hindi
  (0.0% on all 16 PDFs, with and without glyph names; raw text layer 2 to 45%)
- [x] Set C (no `ToUnicode` at all): under 5% (0.0%)
- [x] Bijoy round trip: at least 98% of words exact (99.7% of 3,842 words;
  the rest are words `poriborton` itself cannot encode). Bijoy PDFs: 0.0%
  character errors, also without `ToUnicode`.
- [ ] Text layer for one page under 30 ms; recovery table under 50 ms per font
- [ ] Search finds words in PDFs that the raw text layer misses
- [ ] Robustness test covers the text layer; CI green; PLAN.md updated
- [ ] Review of Phase 2 written and its findings fixed

## Known limits (stated up front)

- A PDF whose font is not among the reference fonts (and keeps no `GSUB`)
  falls back to its `ToUnicode` map with reordering. The app should
  register the phone's Indic fonts; more designs can be bundled later.
- Bare CFF and Type 1 fonts cannot be shaped; outline matching still works
  if a reference font has the same outlines.
- Scanned PDFs have no text layer at all; OCR is out of scope.
