# Review: Phase 5 (ready for the app)

Date: 2026-11-26. Scope: book info and covers, locators, progress,
whole-book search, ranges, sentences, the text-quality hint, the JNI
interface (`app.rs`, `Native.kt`, `check_native.py`, `NativeTest.java`),
the PDF page writer, fuzzing, the phone benchmark, as of commit `b5146f9`
plus the fixes below. Method: line-by-line reading against the
code-review checklist (correctness, concurrency, edge cases, cost,
hostile input), every fix checked to fail its test without it, three
other PDF readers and qpdf on every copied page, eight fuzz targets run
locally and in CI.

Severity: 🔴 must fix (wrong output, crash, hang) · 🟠 should fix ·
🟡 improve or document.

## Evidence

| Check | Result |
|---|---|
| Interface | 54 calls in `Native.kt`, each checked against the library's exports (names, argument and return types) and called from a JVM in CI, with bad handles, closed books, bad locators and six threads at once |
| Info and covers | under 50 ms a book once warm; the first PDF of a process reads the reference fonts (130 to 170 ms on the VM) |
| Locators | survive font size, screen size and theme; progress and its inverse round-trip within 0.01; PDF page and reflow places convert both ways |
| Page copies | 79 test PDFs: copied pages open in MuPDF, PDFium and Poppler with the same text and identical pixels at 72 dpi, and pass `qpdf --check`; the encrypted one is refused ([`page_copies.csv`](../case-study/data/page_copies.csv)). 11 pages of the real 41 MB book: 2.5 MB in 30 ms, identical in MuPDF |
| Fuzzing | 8 targets; locally 1 to 2 minutes each, in CI 60 s each on every push to `main`. Found a hang and a panic (findings 1 and 2) |
| `libakkhor.so` arm64, app build (CI) | 5.89 → 6.01 MB (the interface and the page writer); PDF only 5.26 MB |
| Phone | Realme 9i (Snapdragon 680, Android 13): see "On a real phone" below |

## Findings

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 1 | vendored parser, lenient arrays (Phase 4b) | An unterminated nest of arrays (`[[[[...`) was re-read at every level: page text took 23 s on one fuzzed file and never finished on another. My own Phase 4b patch caused it, by stepping over the bracket of a failed nested array. | 🔴 | Fixed: a failed nested array, dictionary or string fails the array, as upstream. Two fuzzed files and a hand-made nest of 40 are tests; renders of every corpus PDF unchanged. |
| 2 | `akkhor-epub::percent_decode` | A `%` before a multi-byte letter in a path sliced the string inside the letter: panic (CI fuzzing, markup target). A code at the very end of a path was also left undecoded. | 🔴 | Fixed: decoded on bytes. Unit test. |
| 3 | `app.rs`, searches | Closing a book while another thread searched it deadlocked: `close` held the table of searches and waited for a search's lock, while `searchNext` held that lock and waited for the table. The JVM test ran searches and closes, but never at the same time. | 🔴 | Fixed: the table keeps each search's book beside it; a search's lock is released before the table is taken. `NativeTest` closes a book under four searching threads; the old code hung on every run. |
| 4 | PDF text, turned pages | A page turned by `/Rotate` was read as displayed: its lines ran down the page and the text came out one letter per line. Older than this phase; found by the page-writer test file with an inherited `/Rotate`. | 🔴 | Fixed: text, shapes and size are read upright, boxes moved back onto the displayed page. Test checks the boxes against MuPDF's. |
| 5 | reflow, turned pages | A picture on a turned page is drawn as displayed, so in reflow it stays turned. | 🟡 | Open: rare (a landscape table page); rotate the drawn picture. |
| 6 | CI | Two commits went in with a lint error (clippy's output piped into `head` hid its exit status, and a newer clippy in CI flagged what the local one did not). | 🟠 | Fixed; I now check clippy's own exit status before every commit. |
| 7 | fuzz CI | The prebuilt `cargo-fuzz` is a musl build and aimed the sanitizer build at musl: every target failed to compile on the first run. | 🟠 | Fixed: the target is named. |
| 8 | page copies | Links, the outline and form fields' links to other pages are not copied; a copied form widget becomes a field of its own (full name and type from its parents). | 🟡 | Documented in `Native.kt` and `docs/api/android.md`. |
| 9 | errors | An encrypted PDF refused for copying reports "file is DRM-protected", also when it opens with an empty password. | 🟡 | Documented; the app should say "this PDF is protected". |
| 10 | cost | The first PDF opened in a process pays 130 to 170 ms (VM) for the reference fonts, so the library's first card is slower than the rest. | 🟡 | Measured on the phone: 110 to 400 ms, raised to 🟠 as finding 13. |
| 11 | search | A whole-book search of the 2,000-page PDF with no hits takes 3.8 s; it runs in batches the app can stop, but a first hit deep in a large book is slow. | 🟡 | Documented. |
| 12 | fuzzing reach | The PDF targets run 10 to 40 inputs a second (text recovery and reflow are heavy), so 60 s in CI is a shallow pass; the corpus is kept between runs to make up for it. | 🟡 | Documented. |

## What holds up

- Every copied page reads and draws the same in three other engines.
- The interface is checked mechanically: a declaration that drifts from
  the library fails CI, not the app.
- Each 🔴 above has a test that fails (or hangs) without its fix.

## After the review: design answers (5.12)

The app's design chat answered the engine's questions. Applied in the
engine: final theme colours, photos at 85% on dark themes, English
hyphenation rules (6+ letter words, 3 letters each side, at most 2
hyphenated lines in a row, never the last word or the next-to-last
line of a paragraph; a test checks each and fails without the run
limit). Sent back: the font-pack hint should follow the engine's
`needs_fonts` flag (below 0.98), because books that need fonts score
0.94 to 0.95 and a 0.85 threshold would miss them all. Open: no hyphen
across a page break (needs the paginator).

## On a real phone (2026-11-27)

Misbah ran the benchmark on a Realme 9i (Snapdragon 680, Android 13);
data in [`phone.csv`](../case-study/data/phone.csv) and
[`phone_summary.csv`](../case-study/data/phone_summary.csv). The phone is
2 to 3 times slower than the cloud VM.

| Step (median, warm file cache) | Phone | VM |
|---|---|---|
| Open a book | 0.5 ms | 0.1 ms |
| PDF page at full width (1080 px), first time | 30 ms | 15 ms |
| PDF page text | 8 ms | 2 ms |
| Reflowed screen, first draw / again | 17 / 15 ms | 8 / 5 ms |
| Chapter layout | 2 to 6 ms | 1 ms |
| Memory added at peak | 14 to 21 MB | 14 to 27 MB |
| First Indic PDF's card (info), with one-time set-up | 110 to 400 ms | 40 to 150 ms |

New findings:

| # | Finding | Sev. | Status |
|---|---|---|---|
| 13 | The first Bangla or Hindi PDF of a run costs 110 to 400 ms for its card: building the reference fonts' glyph tables, once per process (Chrome-made PDFs need the deepest tables). Later books on the VM: 5 to 16 ms. | 🟠 | App: warm the engine on a background thread at start. Engine (next phase): keep the tables on disk between runs. `akkhor-bench library` now measures a whole library in one process. |
| 14 | Drawing a reflowed screen again costs 15 ms on the phone (5 on the VM): glyphs are rasterised from outlines every time; only the outlines are cached. Fine for page turns (a page is drawn once, ahead), tight for scrolling. | 🟡 | Open: a cache of rasterised glyphs. |

## Result

🔴 four found, all fixed (two by fuzzing, one by review, one by a new
test file). 🟠 6 and 7 fixed; 13 (the first Indic PDF's one-time font
tables, up to 0.4 s on a phone) carried to the next phase: keep them on
disk. 🟡 documented or carried: turned pictures in reflow, slow no-hit
search on huge books, a cache of drawn glyphs (finding 14). Nothing from
the plan is open.
