# Phase 6b: what the app's screens need from the engine

Status: planned (2026-11-27), done after Phase 6's formats and before the
reader app. Source: the finished screen spec (`akkhor-reader-screen-spec.md`
in the claude.ai project, 79 boards) and the design answers, read board by
board for anything the app would have to ask the engine and cannot today.
Done here, the app is written once; after it, interface changes are
additions only.

## Gaps found in the screen spec

| Board | The screen needs | Today |
|---|---|---|
| S05 Appearance | line spacing, margins, font family, justify (left-aligned by default), hyphenation on or off, all in the layout | layout takes width, height and font size; justify is on by default |
| Design answers | links drawn in the link colour with an underline, and a tap on a link goes to its target (footnotes, internal links) | links and anchors are dropped when a chapter is read |
| R08 Figure zoom | which picture a tap hit, its full-size pixels, sharing it as an image | pictures are drawn, not reported |
| X12 TalkBack | one node per paragraph on a screen: its box and text | nothing per paragraph |
| R03, S03, N02 | the table of contents for every format, each entry with a locator and its progress (side bar knots, contents sheet, notes grouped by chapter) | `outlineTitles` and `outlineLevelsAndPages` give PDF pages or chapter numbers |
| X01 DRM, X03 Unsupported, O04 scan | title, author and cover of a protected or not-yet-supported book, and its format, without opening it | opening fails; nothing is returned |
| X02 Damaged, X10 Password | the kind of failure (damaged, protected, password needed, wrong password, unsupported, missing), as a code | `lastError` is an English sentence |
| X10 Password | opening a PDF with a user password | not offered |
| X04, X06, X11 | a fingerprint of a file, to relink a moved file and spot duplicates and changed files | none |
| X11 File changed | finding a stored quote again (text plus a little context) as a locator range | search finds words, not a quote with context |
| S08 Page range | the size of the PDF that chosen pages will make, before making it | none |
| S14 PDF export | real progress while writing, and stopping it (the partial file removed) | `pdfWritePages` runs to the end in one call |
| T07 About | the engine's version | none |

## 6b.1 Layout options

`layoutChapter` takes a JSON options string (additions to it never change
the call): line height, margins, paragraph spacing, first-line indent,
justify (off by default, as designed), hyphenation, font family (a name
registered with `registerFont`), text size. Changing any option keeps the
place (locators do not depend on layout).

## 6b.2 Links and anchors

The chapter reader keeps `<a href>` spans and element ids (EPUB, MOBI,
FB2 notes, TXT none, PDF link annotations in reflow later). Links are
drawn in the theme's link colour with a 1 px underline 3 px below the
baseline. `linkAt(screen, x, y)` returns the target as a locator (inside
the book) or the URL (outside); footnote targets also give the note's
text, for a pop-up.

## 6b.3 Pictures on a screen

`picturesOnScreen(screen)`: each picture's box and an id; `renderPicture(id,
bitmap)` draws it at full size (an EPUB, MOBI or FB2 image, a CBZ page,
or a PDF region in reflow), for figure zoom and for sharing as an image.

## 6b.4 Paragraphs for TalkBack

`paragraphsOnScreen(screen)`: JSON, each paragraph's box, text and
locator range, in reading order, for one virtual node each.

## 6b.5 Contents for every format

`outline(handle)`: JSON entries with title, level, locator and progress,
for PDF (outlines), EPUB (nav or NCX), MOBI, FB2 and CBZ (ComicInfo or
none), so the side bar, contents sheet and notes headers need no format
code in the app. The two PDF-only calls are removed (no app uses them yet).

## 6b.6 Looking without opening

`peek(path)` (and `peekFd`): format, supported or not, protected or not,
title, authors, language and whether a cover exists, for any file the
scanner finds: DRM-protected EPUBs and Kindle files (their metadata is
readable), unsupported containers (KFX, CBR, CB7), damaged files.
`renderPeekCover` draws a cover that is not itself encrypted.

## 6b.7 Errors as codes, passwords

`lastErrorKind()`: none, damaged, protected (DRM), password needed, wrong
password, unsupported, not found, out of range, cancelled. `openPath` and
`openFd` take an optional password; owner-only passwords open silently,
as they do now.

## 6b.8 Fingerprints and quotes

- `fingerprint(path)`: a hash of the size, the first and last 64 KB and,
  for PDFs, the document ID; cheap enough for a whole library scan.
- `findQuote(handle, quote, before, after)`: the locator range of a
  stored quote, using its context to choose among repeats and Indic
  folding as search does; nothing when it is gone.

## 6b.9 PDF export with progress

`pdfEstimateSize(handle, pages)`; `pdfWriteStart(handle, pages, path)` →
an id, `pdfWriteNext(id)` → progress 0 to 1 (a few pages a call),
`pdfWriteCancel(id)` removes the partial file. `pdfWritePages` stays for
small ranges.

## 6b.10 Version

`version()`: the engine's version string (the crate version and the git
commit CI built).

## Done when

- [ ] Layout options (justify off by default, margins, line height, font family, hyphenation)
- [ ] Links: kept, drawn in the link colour with an underline, `linkAt` with targets and note text
- [ ] Pictures on a screen, drawn full size
- [ ] Paragraphs on a screen for TalkBack
- [ ] One outline call for every format, with locators and progress
- [ ] `peek` for protected, unsupported and damaged files
- [ ] Error codes; PDF passwords
- [ ] Fingerprints; `findQuote`
- [ ] PDF export estimate, progress and cancel
- [ ] `version()`
- [ ] Native.kt, JVM test and API doc cover every new call; review; CI green
