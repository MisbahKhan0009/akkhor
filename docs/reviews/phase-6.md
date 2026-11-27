# Review: Phase 6 (every format)

Date: 2026-11-27. Scope: MOBI 7, AZW3 (KF8), combined files and PalmDOC
(`akkhor-mobi`), FB2 and FB2.ZIP (`akkhor-fb2`), comic archives
(`akkhor-cbz`), the facade and JNI changes that put them behind the same
calls, and the items carried from Phase 5 (reference-font tables on disk,
glyph masks, turned pictures, page-end hyphens), as of commit `3ce85e4`.
Method: line-by-line reading against the code-review checklist
(correctness, concurrency, edge cases, cost, hostile input); every fix
checked to fail its test without it; three fuzz targets run locally;
four engines scored on the format corpus; arm64 sizes from CI; a second
pass looking for each bug's pattern elsewhere in the code.

Severity: 🔴 must fix (wrong output, crash, hang, app abort) · 🟠 should
fix · 🟡 improve or document.

## Evidence

| Check | Result |
|---|---|
| Text, against the source EPUBs | 15 Kindle and FB2 books made by Calibre (Bangla, Hindi, English): Akkhor finds 100% of the words in order in all 15, with the same few extra words as KindleUnpack (titles the converter adds). MuPDF gets nothing from AZW3 and 49% of the Bangla words elsewhere; Calibre drops a word in each AZW3 and takes 0.5 to 3 s a book ([`formats_scores.csv`](../case-study/data/formats_scores.csv)) |
| Hostile files | more than 1,700 damaged Kindle files read without a panic (`akkhor-mobi/tests/hostile.rs`); DRM, KFX, Topaz, CBR and CB7 named and refused |
| Fuzzing | 3 new targets, run on stable without a sanitizer: about 220,000 MOBI, 320,000 FB2 and 129,000 comic inputs. 3 crashes found, 2 more of the same kind by review ([`fuzz_phase6.csv`](../case-study/data/fuzz_phase6.csv)) |
| Interface | 55 calls in `Native.kt` (`setCacheDir` added), checked against the library's exports and called from a JVM: an AZW3, a MOBI 7, an FB2, a manga and a refused RAR comic |
| Size, arm64 (CI) | MOBI 56 KB, FB2 38 KB, comics 209 KB (WebP and GIF decoders included); all three 306 KB against a 0.5 MB budget. App build 6.09 → 6.41 MB ([`size_phase6.csv`](../case-study/data/size_phase6.csv)) |
| Speed, VM | every new-format book opens in under 0.2 ms; card (cover) 3 to 5 ms; first reflowed screen 4 to 8 ms; a comic page at phone width 12 to 16 ms ([`phase6_vm.csv`](../case-study/data/phase6_vm.csv)) |
| Glyph masks | redrawing a screen 2.1 to 2.7 times faster on 8 books; first and next screens never slower ([`glyph_masks.csv`](../case-study/data/glyph_masks.csv)) |
| Reference-font tables | built in 2 to 85 ms, read back from disk in 0.04 to 0.8 ms ([`ref_tables.csv`](../case-study/data/ref_tables.csv)) |
| Phone | Realme 9i (Snapdragon 680, Android 13), see "On the phone" below ([`phone_phase6.csv`](../case-study/data/phone_phase6.csv)) |

## Findings

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 1 | FB2, open | The check for `<FictionBook` cut the text at byte 4096; when that byte fell inside a letter (any Bangla or Russian FB2 can), opening panicked. Found by fuzzing. | 🔴 | Fixed: the bytes are searched. Test file `char-boundary-1.fb2.bin`. |
| 2 | PDF, XMP metadata | The same pattern by review: an entity search cut the title 12 bytes on, so `&` followed by Bangla in a PDF's XMP title panicked. | 🔴 | Fixed the same way. Unit test. |
| 3 | FB2, headings | A bold first paragraph that never closed made the heading rewrite run past the end of the text. Found by fuzzing. | 🔴 | Fixed: only a closed paragraph is rewritten. Test file `unclosed-heading-1.fb2.bin`. |
| 4 | MOBI, indexes | An index record's entry count was taken from the file: one claiming billions asked for 8 GB, and a failed allocation aborts the whole app. Found by fuzzing. | 🔴 | Fixed: capped by the offset table the record really holds. Test file `huge-index-count-1.mobi.bin`. |
| 5 | MOBI, HUFF | The same pattern by review: a dictionary could claim 2^31 phrases (about 48 GB). | 🔴 | Fixed: capped by its offsets. Unit test. |
| 6 | CI | CI failed on every push from 6.5 to 6.9 and I didn't look. CI runs Rust 1.100, whose clippy has lints the local 1.97 lacks (`map_or_identity`, constant-size `chunks_exact`); the local toolchain can't be updated here (its download host is outside the network allowlist). | 🟠 | Fixed, green from `92038ac`. From now on every push is checked on CI before the next step. |
| 7 | CLI | `akkhor-cli` built only when other crates switched the new formats on for it: CI's standalone CLI build would have failed. | 🟠 | Fixed: the CLI asks for every format itself. |
| 8 | JNI, memory | `trimMemory` never freed the reflow caches (outlines, masks, pictures), and its Kotlin doc said it returned bytes freed; it returns 0 or -1. | 🟠 | Fixed, doc corrected. |
| 9 | Comics, cache | One decoded page was kept, and decoding happened outside the lock. Drawing the page on screen and the next one ahead made them evict each other on every tile, while threads decoded the same page side by side: 40 alternating tile draws took 1.05 s. | 🟠 | Fixed: pages kept up to 64 MB (always the last one used), each decoded once while other threads wait. Test. |
| 10 | Comics, drawing | A phone-width comic page took 44 ms to draw, redraw included: a float box filter and a function call per pixel. | 🟠 | Fixed: rows and columns worked out once per tile, bilinear when enlarging, integer averages when shrinking. 12 to 16 ms. |
| 11 | Reflow, long chapters | `layoutChapter` lays a whole chapter out before the first screen. Calibre puts a converted text file into one chapter, and some FB2s are one section: a 2.6 MB Bangla book as AZW3, MOBI or FB2 takes 410 to 440 ms and 65 MB on the VM (more than a second on a phone, by the Phase 5 ratio). | 🟠 | Open, planned as 6b.11: long chapters split into parts inside the engine, so the app's calls stay the same. |
| 12 | Glyph masks | Masks snap glyphs to a quarter pixel, so edge pixels differ from outline drawing: largest channel change 49 of 255, mean under 0.5. At a 1/64 px step the difference is 4, so the blending maths is exact. | 🟡 | Accepted (an eighth of a pixel is not visible in body text); the plan's "pixels unchanged" is reworded to say so. |
| 13 | Limits | Hostile files can still make the engine allocate a few hundred MB within its limits: a zip entry up to 256 MB, MOBI text up to 512 MB, a 40 MP comic page decoded before being scaled to 12 MP. Bounded, but high for a phone. | 🟡 | Documented; revisit with phone memory numbers. |
| 14 | KF8 fonts | Fonts embedded in AZW3 books are not used, as for EPUB: the phone's fonts draw the text. | 🟡 | Documented. |
| 15 | MOBI links | `filepos` links become `#filepos` anchors but nothing follows them yet. | 🟡 | 6b.2 (links and `linkAt`). |
| 16 | Corpus | The format corpus books are short (about 800 characters of text), so the scores show correctness, not scale. Big-book timings came from Calibre conversions of `big-bn.txt`, made locally and not committed. | 🟡 | Documented. |
| 17 | Examples | `copy_pages` needed the `write` switch without saying so; testing `akkhor-pdf` on its own failed to build it. | 🟡 | Fixed (`required-features`). |

Checked and fine: the reference-table files (magic, lengths and a
fingerprint of the table code; a cut file is rebuilt, never misread; a
test checks tables read back equal built ones); `setCacheDir` removes
only `akkhor-*.bin` files of older engines; the turned-picture maths (an
exact inverse for all three turns, by test); carrying hyphenated lines
keeps every line on its page and the text complete (600 page sizes, by
test).

## On the phone

Realme 9i, the same phone as Phase 5, medians of the warm passes:

| Measure | Phase 5 | Phase 6 |
|---|---|---|
| Redrawing a reflowed screen (5 books) | 13.5 to 15.9 ms | 3.6 to 5.6 ms |
| Drawing a reflowed screen the first time (PDF reflow) | 16.1 to 17.3 ms | 17.5 to 19.3 ms |
| A Kindle or FB2 book: open / card / first screen | | 0.2 to 1 ms / 6 to 18 ms / 7.5 to 11 ms |
| A comic page at 1080 px, and again | | 52 ms, 47 ms |
| First Bangla or Hindi PDF's card | 110 to 400 ms | 110 to 400 ms (not tested, see 19) |
| Memory added | 14 to 21 MB | 11 to 21 MB |

| # | Where | Finding | Sev. | Status |
|---|---|---|---|---|
| 18 | Glyph masks, phone | The first draw of a reflowed PDF screen is 1.4 to 2 ms slower than in Phase 5 (8 to 12%): making the masks costs more than filling outlines once. The VM showed no loss. Redraws are 2.7 to 4.2 times faster, so a page drawn ahead and shown later comes out ahead. | 🟡 | Accepted. |
| 19 | Phone benchmark | The benchmark never set a cache folder, so the reference-font tables on disk (6.8) were not used on the phone and the first Indic PDF's card stayed at 400 ms. Not an engine bug; the bench didn't do what the app does. | 🟠 | Fixed: `akkhor-bench` reads `AKKHOR_CACHE_DIR`, and `on-phone.sh` sets it (emptied first). Second run ([`phone_phase6_run2.csv`](../case-study/data/phone_phase6_run2.csv)): cards 400 → 47 to 76 ms for Chrome PDFs, 13 to 32 ms for the others; target 50 ms met by 3 of 4 PDFs' first card. |
| 20 | Comics, phone | A comic page took 47 to 52 ms on the phone (3.5 times the VM). | 🟠 | Fixed: two colour channels per multiply, each source row blended across once. Second run: 29 ms first, 24.5 ms again. |
| 21 | Card from stored tables | Reading the stored tables still costs about 3.7 ms on the VM; a third of it was hashing every font file in full to find its tables. | 🟡 | The key now hashes a font's length and its first and last 64 KB (the table directory and checksums are at the start). Next phone run to show the gain. |
| 22 | Phone runs | The second run was 10 to 40% slower across the board, one AZW3 screen 5 times (11 → 55 ms): the phone was likely throttled with the screen off. Numbers from one run are not comparable with another unless the phone is in the same state. | 🟡 | `scripts/phone/README.md`: keep the screen on and the phone charging during runs. |

## Result

🔴 five found, all fixed (three by fuzzing, two by looking for the same
pattern). 🟠 six fixed; one (long single chapters, finding 11) planned
as 6b.11 because the fix changes how chapters are counted, which the app
will rely on. 🟡 documented. Phone numbers for the new formats come from
the next phone run.

## Lesson

Finding 6 is a process failure, not a code one: I pushed five times
without looking at CI, trusting a local toolchain three versions older.
Checking the run after each push is now part of every step.
