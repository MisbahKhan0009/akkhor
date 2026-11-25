# Akkhor on Android: the interface

The engine is `libakkhor.so` (built by CI for arm64-v8a, armeabi-v7a and
x86_64) and one Kotlin file, [`android/Native.kt`](../../android/Native.kt),
which the app copies as-is. This page explains how the calls fit together.
CI checks every declaration in `Native.kt` against the library
(`scripts/check_native.py`) and calls each one from a JVM
(`android/test/NativeTest.java`).

Interface version: **1** (`Native.apiVersion()`). It changes only when a
call changes meaning or signature; new calls do not change it. An app should
refuse to run with an engine whose version differs from `Native.API_VERSION`.

## Rules for every call

- **Threads:** any call may come from any thread. Calls on one open book
  run one at a time; calls on different books run in parallel. A long PDF
  render on a book delays a hit test on the same book: `cancelRendering`
  stops a batch of tiles before its next tile.
- **Errors:** calls return `null`, `-1` or `0` as their documentation says,
  and `lastError()` (per thread) says why. A damaged or hostile file never
  crashes the app: panics are caught at the boundary.
- **Handles:** `openPath` or `openFd` give a `Long` handle; `close` frees it.
  Handles are never reused, so a call with a closed handle is a plain error.
  `close` while another thread is in a call on that book lets the call
  finish first. Closing a book drops its searches.
- **Text offsets:** engine offsets are UTF-8 bytes. Kotlin strings count
  UTF-16 units: convert with `utf16Offset` and `utf8Offset`. Search
  snippets already give their match in UTF-16 units.
- **JSON:** calls that return structured data return one JSON string. New
  fields may appear in later versions; ignore fields you do not know.

## Start-up

```kotlin
check(Native.apiVersion() == Native.API_VERSION)
// The phone's fonts: used for drawing, and to read PDFs that embed
// stripped copies of them.
Native.registerFont("Latn", "/system/fonts/NotoSerif-Regular.ttf")
Native.registerFont("Beng", "/system/fonts/NotoSerifBengali-Regular.ttf")
Native.registerFont("Deva", "/system/fonts/NotoSerifDevanagari-Regular.ttf")
Native.registerStandardFont("Helvetica", "/system/fonts/Roboto-Regular.ttf")
// The design's reading colours (ARGB), then pass 16, 17, ... as `theme`.
Native.setThemeColors(16, 0xFFF7F3EA.toInt(), 0xFF221F1A.toInt())
```

Built-in themes: 0 paper, 1 sepia, 2 dusk, 3 night, 4 original (PDF
colours untouched). Codes 16 to 63 are the app's.

## The library

`info(handle)` returns:

```json
{"format":"pdf","title":"নদীর ধারে একটি গ্রাম","authors":[],"language":"bn",
 "language_guessed":true,"publisher":null,"length":4,"has_cover":true,
 "reflowable":true,"text_quality":1.0000,"needs_fonts":false,"bijoy":false}
```

| Field | Meaning |
|---|---|
| `title` | from the book; for PDFs without one (or with a file name as title), the largest line of the first page; for text files, the first line. `null`: show the file name |
| `authors`, `publisher` | from the book (EPUB metadata, PDF document info) |
| `language` | from the book, else guessed from the script of the first text (`language_guessed`) |
| `length` | PDF pages, EPUB chapters, 1 for a text file |
| `has_cover` | EPUBs that name a cover image; every PDF (its first page) |
| `reflowable` | EPUB and text always; PDFs with real text (not scans) |
| `text_quality`, `needs_fonts` | PDFs with Bangla or Devanagari text: how sure the text is (0 to 1). `needs_fonts` when below 0.98: some conjuncts cannot be read without the book's font; offer the free font pack |
| `bijoy` | a text file typed in Bijoy, shown converted to Unicode |

`renderCover(handle, bitmap)` draws the cover fitted into the bitmap (top
left) and returns `(width shl 32) or height` of what it drew, or `0` when
there is no cover (draw a placeholder). Info and cover take under 50 ms a
book once the engine is warm; the first PDF of a session also reads the
reference fonts once (a few hundred ms).

## Reading and saving the place

Two views:

- **Page view** (PDF): `renderPdfTile` / `renderPdfTiles` draw tiles of a
  page at any zoom; `pdfHitTest`, `pdfWordAt`, `pdfSelectionRects`,
  `pdfTextBetween`, `pdfSearch` work on its text.
- **Reflowed text** (EPUB, TXT, PDF "Reflow"): `layoutChapter(handle,
  chapter, width, height, fontSize)` lays out one chapter into screens;
  `renderFlowPage` draws a screen; `hitTest`, `wordAt`, `selectionRects`,
  `textBetween`, `search` work on the laid-out chapter. `chapterCount`
  gives the chapters.

**Locators** are the one value to store for a place:
`akkhor:1/c3/b12/o340` in reflowed text, `akkhor:1/p7/o0` on a PDF page.
They do not depend on layout, so a saved place survives font size, screen
size and theme changes.

```kotlin
// Save: where the current screen starts.
val place = Native.locatorOfScreen(book, screen)
// Restore: lay out its chapter, then find its screen.
Native.layoutChapter(book, chapterOf(place), w, h, fontSize)
val screen = Native.screenOfLocator(book, place)
// Scrubber and "time left".
val progress = Native.progressOf(book, place)        // 0 to 1
val there = Native.locatorAtProgress(book, 0.42)
// PDF: switch views at the same place.
val inReflow = Native.convertLocator(book, "akkhor:1/p7/o120")
```

`clampLocator` brings any stored locator inside the book (a place past the
end opens at the end). Progress counts EPUB chapters by their size and PDF
chapters by their pages, places inside a chapter by text.

## Highlights, notes, copy and quote cards

A range is two locators. `textOfRange(handle, start, end)` gives its text
across chapters or pages; `rectsOfRange(handle, screen, start, end)` gives
the boxes to paint on a screen of the laid-out chapter (pixels) or on the
PDF page (points). Boxes always cover whole syllables. A range saved in one
view can be shown in the other with `convertLocator` on both ends.

## Read aloud

`sentences(handle, from, count)` returns the next sentences from a place:

```json
[{"start":"akkhor:1/c0/b1/o0","end":"akkhor:1/c0/b1/o58","text":"ভাষা আমাদের পরিচয়ের সবচেয়ে গভীর চিহ্ন।"}]
```

Sentences end at `.`, `?`, `!`, `।` and `॥` (not at abbreviations such as
"Dr." or initials) and never cross a paragraph or heading. Speak `text`
with Android's `TextToSpeech`; its `onRangeStart` gives UTF-16 ranges
inside `text`, which `utf8Offset` turns into byte offsets to add to the
sentence's `start` for a word highlight. PDF page places are read in the
reflowed text (without running headers).

## Search

```kotlin
val id = Native.searchStart(book, query)
while (true) {
    val batch = Native.searchNext(id, 20) ?: break   // JSON
    show(batch)                                       // hits, done, searched
    if (batch.contains("\"done\":true")) break
}
Native.searchCancel(id)   // when the user leaves; harmless after done
```

Each hit: `start`, `end` (locators), `snippet`, `match_start`,
`match_end` (UTF-16 units in the snippet), `at` (chapter or page),
`in_picture` (the text is in a picture: a wide table or a label; show the
picture, not a highlight). Hits come in book order; `searched` (0 to 1) is
how much of the book has been looked at. EPUBs and text files are searched
by chapter in reflowed text; PDFs by page in page text, so their hits are
page places (`convertLocator` for the reflow view). Indic spellings fold
(nukta letters, split vowels, ৎ, ZWJ/ZWNJ), and a word split by a line-end
hyphen is found.

## Sharing pages of a PDF

`pdfWritePages(handle, pages, path)` saves chosen pages (0-based, in the
given order) as a new PDF, for "share this chapter" or "send these pages".
Each page is copied with everything it uses, streams untouched, so its
text stays selectable and the file is about the size of what the pages
use (11 pages of a 41 MB book: 2.5 MB, 30 ms). Highlights, notes and
drawn form fields come along; links and the outline do not. Encrypted
PDFs are refused, even those that open without a password. Write to the
app's cache directory, then share the file.

## Memory

`trimMemory(handle)` frees caches (fonts, glyphs, page text, images) and
returns the bytes freed: call it from `onTrimMemory`. Every cache has a
byte budget; a 308 MB, 2,000-page PDF stays near 30 MB.
