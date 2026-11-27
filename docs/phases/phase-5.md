# Phase 5: ready for the app

Status: planned (2026-11-26). The reader app (its own repository, designed
in a separate chat) starts once this phase is done. This phase gives it
everything it needs from the engine, behind an interface that stops
changing, hardened against the files real people have. More formats move
to Phase 6, more scripts to Phase 7, release work to Phase 8.

## Starting point (2026-11-26)

What the app plan (`akkhor-reader-app-plan.md` in the claude.ai project)
needs, against what the JNI bridge (`dev.akkhor.Native`, 36 calls) has:

| The app needs | Today |
|---|---|
| Library: title, author, language, cover, length, for every book, fast | EPUB title and language inside `akkhor-epub`, not exposed; no author or cover; PDF document info not read; nothing for TXT |
| A saved place that survives font size, theme and view changes | positions are (block, byte) inside one chapter; no single value to store; no progress (0 to 1) for the scrubber or "time left" |
| Search across the whole book, with snippets, stoppable | per chapter (flow) and per page (PDF), no snippets over JNI, no way to stop |
| Highlights and notes that stay put | selection works per screen; no stored range, no "text of this range" across chapters |
| Read aloud | UTF-16 offsets only; no sentences |
| Share pages of a PDF as a PDF | the engine cannot write PDFs |
| The designer's colours | themes are four fixed codes |
| A stable, documented interface | 36 calls documented in a doc comment only; nothing checks that the Kotlin side matches |
| Files from the wild | robustness tests mutate 3 book PDFs; no fuzzing |
| A phone | every number measured on a cloud VM |
| A hint when a PDF's fonts are missing | nothing (68% vs 100% on the real book depends on it) |

## 5.1 Book info for the library

`info(handle)`: format, title, authors, language, publisher, page or
chapter count, whether a cover exists, and for PDFs whether pages can be
reflowed and how readable the text is (5.9). Sources: EPUB OPF
(`dc:title`, `dc:creator`, `dc:language`, `dc:publisher`, cover from
`properties="cover-image"` or `meta name="cover"`), PDF document info
dictionary and XMP title, TXT the first non-empty line. `renderCover`
draws the EPUB cover image, the first PDF page, or nothing (the app draws a
placeholder), into a bitmap of the size asked. Target: info and cover of a
book in under 50 ms, without reading chapters or pages beyond the first.

## 5.2 Positions that survive everything

One **locator** string per place, the only thing the app stores:
`akkhor:1/c<chapter>/b<block>/o<byte>` for EPUB, TXT and PDF reflow,
`akkhor:1/p<page>/o<byte>` for the PDF page view, convertible both ways
for PDFs. Calls: `locatorAt(screen)`, `screenOf(locator)`, `progressOf(locator)`
(0 to 1, by text length), `locatorAtProgress(f)` for the scrubber. A
locator from an older engine version still opens near the same place;
one that points past the end opens at the end. Tested by changing font
size, screen size and theme with locators stored before.

## 5.3 Whole-book search

`searchStart(handle, query)` returns a search id; `searchNext(id, max)`
returns up to `max` hits (locator, snippet with the match marked, chapter
or page); `searchCancel(id)`. Runs chapter by chapter or page by page, so
the first hits come at once on a 2,000-page PDF; the Indic folding of
Phase 1 applies, and the hyphen-aware page search of Phase 4b. Hits in a
picture's alt text say so (the app shows the picture, not a highlight).

## 5.4 Ranges: highlights, notes, copy, quote cards

A range is two locators. `textOf(start, end)` across chapters or pages;
`rectsOf(start, end)` on the screen or page shown (snapped to syllables,
as selection already is); a range made in the page view shows in the
reflow and the other way round. Stored ranges survive a font size change
(tested).

## 5.5 Read aloud

`sentences(handle, from, count)`: the next sentences from a locator, each
with its text, its locator range and UTF-16 offsets, ending at `.`, `?`,
`!`, `।`, `॥` and paragraph ends (not at abbreviations such as "Dr." in
English). Android's offline voices read the text and report UTF-16 offsets;
`rectsOf` highlights the word being read.

## 5.6 Pages of a PDF as a new PDF

`pdf-write` switch (on in the app build): copies chosen pages and
everything they use (fonts, images, resources) into a new PDF, renumbering
objects; streams are copied as they are (no recompressing), so text stays
selectable and searchable and the file is small. Pages from encrypted PDFs
are refused. Tested: the copy opens in Akkhor, MuPDF, PDFium and Poppler,
with the same text and the same pixels as the original pages.

## 5.7 One documented interface

- **Themes from the app:** any background, text, link and highlight
  colours (the designer's palette), not four codes; the four presets stay
  as defaults.
- **The interface written down:** `docs/api/android.md` (every call, its
  arguments, errors, which thread may call it) and `android/Native.kt`, the
  Kotlin declarations the app copies. `apiVersion()` returns the interface
  version; the app refuses an engine it does not know.
- **Structured results as JSON** (book info, outline, search hits,
  sentences): one string per call, easy to extend without breaking.
- **Threads:** rendering, text and search may run on different threads at
  once on one book; `close` waits for them. Written down and tested.
- **Checked:** a JVM test loads the host build of `libakkhor` and calls
  every function through `Native.kt`, so a mismatched signature fails CI,
  not the app.

## 5.8 Fuzzing

`cargo-fuzz` targets: PDF open and page text and a small render, PDF
reflow, EPUB open and a chapter, the zip reader, TXT decoding and Bijoy
detection, CSS and markup, locator parsing, the PDF writer. Seeds from
`tests/corpus`. CI runs each target for a fixed time on every push to
`main`; every crash found becomes a test before it is fixed.

## 5.9 Missing-font hint

For PDFs: the share of Indic glyphs on the first pages that were read from
a font the engine has (embedded with rules, or a registered reference),
reported in `info` as `text_quality` (0 to 1). Below a threshold the app
can offer the free Bangla and Hindi font pack. Tested on the corpus with
and without the reference fonts.

## 5.10 A phone

`akkhor-bench` built for arm64 Android by CI, with instructions to run it
on a phone over `adb` (or Termux). Misbah runs it on a phone; the numbers
(cold open, first page, redraw, reflow chapter, memory) go into the case
study next to the VM numbers. This step needs the phone; everything else
does not.

## 5.11 Review

Case study, CSVs, `history.csv`; review in `docs/reviews/phase-5.md`.

## 5.12 Design answers (added 2026-11-26)

The design chat answered the engine's questions (project doc
`design-answers-for-engine.md`). What falls to the engine:

1. **Built-in themes take the final values**: Paper `#F7F3EA` / `#2A2620`,
   Sepia `#F1E4CC` / `#433422`, Dusk `#2A2825` / `#DDD3C2`, Night
   `#000000` / `#B8B2A6`. The app can still pass its own (codes 16 to 63).
2. **Photos at 85% brightness in Dusk and Night**, in PDF page view and in
   reflow pictures; line art keeps the theme remap.
3. **Latin hyphenation rules**: words of 6 or more letters, at least 3
   letters before and 3 after the break, at most 2 hyphenated lines in a
   row, never the last word of a paragraph. Bangla and Devanagari never
   hyphenate (already so). "Never across a page" needs the paginator and
   is left open.
4. **Font-pack trigger**: the design proposed text quality below 0.85;
   measured books that need fonts score 0.94 to 0.95, so the app should
   use the engine's `needs_fonts` flag (below 0.98). Sent back to the
   design chat.

Selection, highlights, links and search marks are drawn by the app over
the engine's boxes; placeholder covers, the page-view ground and the
Page / Reflow switch are the app's.

## Done when

- [x] Book info and covers for EPUB, PDF and TXT, under 50 ms a book
- [x] Locators, progress and the scrubber; survive font, screen and theme changes
- [x] Whole-book search with snippets, incremental and stoppable
- [x] Ranges: text, rects, across views; survive font changes
- [x] Sentences for read aloud (Bangla, Hindi, English)
- [x] PDF page-range writer, checked by four engines
- [x] Colour themes from the app; `docs/api/android.md`, `Native.kt`, `apiVersion`; JVM test of every call
- [x] Fuzz targets in CI; every crash found fixed with a test
- [x] Missing-font hint
- [ ] Phone numbers (with Misbah's phone): the benchmark is built by CI, waiting for a run
- [x] Case study, review, CI green
- [x] Design answers (5.12)

## Risks

| Risk | Mitigation |
|---|---|
| The interface still needs changes once the app is built | `apiVersion`, JSON results that can grow; changes after this phase are additions |
| PDF writing is a new kind of code (output, not input) | only copying objects, no editing; checked by four other engines |
| Fuzzing needs a nightly Rust compiler | only in CI; the stable robustness tests stay |
| No phone in the cloud | the benchmark is a binary Misbah runs; nothing else waits for it |
