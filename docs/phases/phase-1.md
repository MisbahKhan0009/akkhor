# Phase 1: Indic text core (detailed plan) ✅ done

Goal: make reflowable text (EPUB, TXT) complete and correct, with every
operation aware of Indic syllables. This file is the design; `docs/PLAN.md`
holds the roadmap and status.

Order of work, one commit (or a few) per step, CI green after each.

---

## 1.1 Positions, hit-testing and selection

**Why:** a reader taps, long-presses and drags. None of these may land inside
a conjunct (W3C: a conjunct is one unit for editing operations).

**Design** (`akkhor-flow/src/select.rs`, `model.rs`, `layout.rs`):

- `Position { block, offset }` (byte offset in the block text), ordered.
- Layout keeps, for every placed glyph, its cluster (byte offset) and
  advance, and for every line its top, bottom and x extent.
- `hit_test(page, x, y) -> Option<Position>`: nearest line by y, nearest
  cluster edge by x, snapped to an orthographic-syllable boundary.
- `word_at(doc, pos) -> Range<Position>`: long-press selection; words end at
  spaces and punctuation (including । ॥).
- `selection_rects(page, start, end) -> Vec<Rect>`: one rectangle per line
  piece, edges snapped to syllables. Used for selection and highlights.
- `text_between(doc, start, end) -> String` for copy.

**Tests:** hit-testing anywhere over ক্ষ, ন্ত্র, क्ष returns the syllable start;
rectangles never cut a conjunct; copy round-trips.

## 1.2 Indic-aware search

**Why:** the same word reaches books in several encodings; a search that
compares code points misses matches, or matches half a conjunct.

**Design** (`akkhor-flow/src/search.rs`):

- `normalize(text) -> (String, Vec<usize>)`: folded text plus a map back to
  original byte offsets. Folding:
  - decompose nukta letters: Devanagari क़ ख़ ग़ ज़ ड़ ढ़ फ़ य़ ऩ ऱ ऴ, Bangla ড় ঢ় য়
    (precomposed and base + nukta compare equal);
  - decompose Bangla split vowels ো ৌ;
  - drop ZWJ, ZWNJ, soft hyphen; NBSP becomes space; whitespace collapses;
  - khanda ta ৎ equals ত + hasant (+ ZWJ);
  - Latin letters lower-cased.
- `find_all(doc, query) -> Vec<Match>`. A match must start at a syllable
  start and must not end before a hasant/halant or nukta, so "ক" finds কি
  but not the first half of ক্ষ.
- `EpubBook::search(query)`: chapter by chapter, returns (chapter, match);
  the caller can stop early.

**Tests:** precomposed vs decomposed nukta, ৎ vs ত্‍, ZWJ/ZWNJ variants, case,
no matches inside conjuncts, offsets map back exactly.

## 1.3 Line-breaking rules and hyphenation

**Design** (`layout.rs`, new `hyphen.rs`):

- No break before closing punctuation (। ॥ , . ; : ! ? ) ] } » ” ’) and no
  break after opening punctuation (( [ { « “ ‘), even across a space.
- English hyphenation with Liang's algorithm and the hyph-utf8 `en-us`
  patterns (31 KB, compiled in behind the `hyphenation` feature, on by
  default). Only Latin words, at least 2 letters before and 3 after the
  break. A hyphen glyph is drawn at the line end.
- Indic words are not hyphenated (not the convention); too-long words still
  break between syllables.

**Tests:** "শব্দ ।" never starts a line with ।; known hyphenations (ta-ble,
as-so-ciate, hy-phen-ation); justified Latin lines get tighter.

## 1.4 Font fallback and bold/italic

**Design** (`fonts.rs`, `shape.rs`, `paint.rs`):

- Runs are split by font coverage: a character the script's font lacks goes
  to the next font that has it (marks stay with their base).
- Fonts are registered with a style (regular, bold, italic, bold italic).
  Style spans split shaping runs. When the style is missing, synthetic bold
  (outline stroke) and synthetic italic (12° slant) are used.

**Tests:** a Bangla run containing a Latin-only symbol falls back; bold
spans produce bold glyphs; missing bold falls back to synthetic.

## 1.5 CSS subset

**Design** (new `css.rs`, `model.rs`):

- Sources: `<style>` blocks, linked stylesheets (EPUB passes their text in),
  `style=""` attributes.
- Selectors: element, `.class`, `element.class`, `#id`, comma lists. Others
  are ignored safely. Specificity and order follow CSS.
- Properties: `text-align`, `text-indent`, `margin-top`, `margin-bottom`,
  `font-size` (em, %, px, keywords), `font-weight`, `font-style`,
  `display: none`, `page-break-before` / `break-before`.
- Result: a `BlockStyle` per block and bold/italic spans; layout honours them.

**Tests:** centred headings, indents, hidden elements, page breaks, inline
`style` overriding classes.

## 1.6 Images in EPUB

**Design** (`model.rs`, `layout.rs`, `paint.rs`, `akkhor-epub`):

- `<img>` and SVG `<image>` become image blocks (source path + alt text).
- An `ImageProvider` trait gives an image's size (read from the file header
  only: PNG, JPEG, GIF, WebP dimensions) and decodes it on demand.
- Layout scales images to fit the text width and the page height; an image
  that does not fit the rest of the page moves to the next page.
- Painting decodes at the displayed size and caches pixels in a byte-budget
  cache. Decoders: `png` (already in the build through the rasterizer) and
  `zune-jpeg` (already in the build through the PDF switch).

**Tests:** header sizes for PNG/JPEG/GIF; a tall image gets its own page;
decode and draw a fixture image.

## 1.7 Incremental pagination

**Design:** `Paginator` yields pages one at a time and keeps its place, so
the first page is on screen before the chapter is fully laid out; the rest
can continue on a background thread and stop at any time. `paginate()`
becomes `Paginator::collect()`. `page_for(position)` lays out only as far as
needed to find a saved position.

**Tests:** paginator output equals `paginate()`; first-page time is
independent of chapter length.

## 1.8 Text files: encodings and Bijoy detection

**Design** (`akkhor` `txt` path, new `encoding.rs` in flow):

- Decode UTF-8 (with or without BOM), UTF-16 LE/BE (BOM), else Windows-1252.
- Detect Bijoy/SutonnyMJ-encoded Bangla (ASCII-range "Bangla" such as
  `Avwg` for আমি) by character and word statistics, and report it as
  `TextEncoding::LegacyBijoy` so the app can tell the user. Conversion to
  Unicode lands in Phase 2 with the PDF converters.

**Tests:** each encoding round-trips; Bijoy sample detected; English
Windows-1252 text not flagged.

## 1.9 Android bridge and CLI

Expose the new pieces: `hitTest`, `selectionRects`, `textBetween`,
`search` (per chapter), and a `--search` flag in the CLI. Update the README.

## Done when

- [x] Selection, hit-testing and highlights never split a conjunct (tests in 9 scripts)
- [x] Search finds every variant spelling in the test corpus and nothing inside conjuncts
- [x] Every CSS property above changes layout as expected
- [x] EPUB images render (PNG, JPEG; GIF/WebP are sized only)
- [x] First page of a 1,000-page Bangla text is laid out in under 30 ms on a laptop: 0.75 ms (phone measurement waits for the app)
- [x] CI green, PLAN.md updated with results
