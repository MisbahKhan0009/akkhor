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

For every embedded TrueType/OpenType font with a `cmap`, build a table
glyph id → Unicode text, once per font per document:

1. Reverse the `cmap`: glyph → the character that selects it.
2. Run the font's own OpenType rules forwards on candidate syllables and
   record what comes out: for every consonant pair `C ্ C`, common triples,
   reph `র্ C` / `र् C`, phala forms (`C ্ য/র/ব`), consonant + each vowel
   sign, nukta forms. Each glyph produced only by a candidate maps back to
   that candidate's text. Shaping is HarfRust (already in the engine) with
   the embedded font, so the result is exactly what the producer drew.
3. Mark glyph roles: pre-base vowel sign, reph, below-base, so ordering can
   be repaired.

Budget: under 50 ms per font, cached per document.

Choosing the text for a glyph: a trustworthy `ToUnicode` entry (not empty,
not `\0`, not private use, not `?`) unless the recovered entry is longer
(a conjunct the map cut short); otherwise the recovered entry; otherwise the
glyph name (`uni0995`, `ka-deva`, Adobe Glyph List); otherwise nothing.

## 2.3 Visual to logical order

Glyphs come in drawing order. Per line, per script:

- a pre-base vowel sign (Bangla ি ে ৈ, Devanagari ि, and the Gurmukhi,
  Gujarati, Odia, Tamil, Telugu, Kannada, Malayalam equivalents) moves after
  the consonant cluster that follows it;
- two-part vowels are rejoined (ে … া → ো, ে … ৗ → ৌ);
- a reph drawn at the end of its syllable moves to the start;
- the result is checked with the Phase 1 syllable rules.

## 2.4 Lines, words, reading order

Glyphs are grouped into lines by baseline (tolerance relative to font size),
sorted left to right, spaces inserted where gaps exceed a fraction of the
font size, blocks split at large vertical gaps, and simple two-column pages
read column by column. Output: `PageText { text, char boxes, lines }`, where
every character of `text` points back to a box on the page.

## 2.5 Legacy encodings

- **Bijoy** (SutonnyMJ and other "MJ" fonts): detected by font name or by the
  Phase 1 text statistics; converted with a table derived from the MIT
  licensed `poriborton` crate (OpenBangla), inverted, plus reordering of
  e-kar/i-kar written before the consonant and of the reph written after
  it. Also applied to Bijoy `.txt` books (Phase 1 only detected them).
- **Kruti Dev / Chanakya** (Hindi): detected by font name. The table is
  written for Akkhor from the Kruti Dev 010 layout and checked against
  published word pairs; marked experimental until a real corpus confirms it.

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

Metric: character error rate (edit distance / length) after normalisation.
Bijoy conversion is also checked by round trip: Unicode → Bijoy (with
`poriborton`, test-only dependency) → Akkhor → Unicode.

## Done when

- [ ] Sets A and B: character error rate under 2% for Bangla and Hindi
- [ ] Set C (no `ToUnicode` at all): under 5%
- [ ] Bijoy round trip: at least 98% of words exact
- [ ] Text layer for one page under 30 ms; recovery table under 50 ms per font
- [ ] Search finds words in PDFs that the raw text layer misses
- [ ] Robustness test covers the text layer; CI green; PLAN.md updated
- [ ] Review of Phase 2 written and its findings fixed

## Known limits (stated up front)

- Fonts embedded without a `cmap` (some CID-keyed subsets) cannot be
  reverse-shaped; they fall back to `ToUnicode` and glyph names. Matching
  glyph outlines against reference fonts is a possible later step.
- Scanned PDFs have no text layer at all; OCR is out of scope.
