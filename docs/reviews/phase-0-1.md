# Review: Phases 0 and 1

Date: 2026-11-25. Scope: every crate as of commit `06e745f`
(Phase 1 done). Method: line-by-line reading plus new evidence:
a robustness test (`crates/akkhor/tests/robustness.rs`) that feeds 820
damaged files and 35 random inputs to every parser and renderer, timing of
hostile inputs, and failing unit tests written for each suspected bug.

Severity: 🔴 must fix (crash, undefined behaviour, wrong output) ·
🟠 should fix (performance cliffs, fragile behaviour) · 🟡 improve.

## Evidence

| Check | Result |
|---|---|
| 400 damaged EPUBs, 320 damaged PDFs, 100 damaged texts, 35 random inputs | no panics |
| 20,000 nested `<div>` | **8.7 s** to parse (quadratic) |
| 25,000-line text file with no blank lines | becomes **one** 200,000-word paragraph; first page 786 ms |
| `<style/>` (self-closing) in `<head>` | **all following text disappears** (failing test) |
| empty `<div style="page-break-before:always">` | page break **lost** (failing test) |

## Findings

| # | Where | Finding | Sev. |
|---|---|---|---|
| 1 | `akkhor-android` | A handle is a raw pointer turned into `&mut Handle` on every call. Two threads (a render thread and the UI thread doing hit-testing) calling at once create two `&mut` to the same data: undefined behaviour. A call after `close`, or a double `close`, is a use-after-free. | 🔴 |
| 2 | `akkhor-android` | Strings cross JNI as "modified UTF-8" (`GetStringUTFChars`, `NewStringUTF`), which encodes characters outside the BMP differently from UTF-8. Copying text with an emoji can hand the JVM invalid modified UTF-8 (CheckJNI aborts the app); queries and paths with such characters arrive mangled. | 🔴 |
| 3 | `flow::model` | A self-closing `<style/>` starts CSS capture that never ends: the rest of the chapter is swallowed. | 🔴 |
| 4 | `flow::model` | A page break on an empty element is carried, then overwritten when the next block opens. | 🔴 |
| 5 | `flow::model` | Every element clones the whole ancestor stack for CSS matching: O(elements × depth); 20,000 nested divs take 8.7 s. No depth limit. | 🟠 |
| 6 | `flow::model` | Plain text with one paragraph per line (common for Bangla `.txt`) becomes a single giant block: slow first page, huge positions, search snippets across the whole file. | 🟠 |
| 7 | `flow::layout` | Line breaking scans the break list from the start for every line and re-sums glyph advances for every candidate: O(breaks × lines) per paragraph. | 🟠 |
| 8 | `core::LruCache` | Eviction scans the whole map for the oldest entry: O(n) per insert once full (CJK and large books have thousands of glyphs). | 🟠 |
| 9 | `akkhor-pdf` | hayro is young; a panic inside it on a malformed PDF unwinds into the caller. The Android bridge catches it, but Rust users of the library do not get an `Error`. | 🟠 |
| 10 | `epub::zip` | Offsets and sizes from the file are added without overflow checks (debug builds panic, 32-bit builds can wrap). Stored (uncompressed) entries are copied whole with no size limit. | 🟠 |
| 11 | `flow::image` | Decode limit of 40 megapixels allows a 160 MB temporary buffer, too much for a "lightweight" engine on a phone. | 🟠 |
| 12 | `epub` | DRM check flags a book as protected unless `encryption.xml` contains the word "fonts" anywhere: font-obfuscated books whose font folder is named differently are rejected, and real DRM that mentions "fonts" slips through. | 🟠 |
| 13 | `flow::shape` | A HarfRust shape plan is built for every run of every paragraph; plans are reusable per font and script. | 🟡 |
| 14 | `flow::layout` | An image taller than the page area with top margin larger than the page gives a negative size. | 🟡 |
| 15 | `akkhor-pdf` | A new render cache per tile: fonts and images are re-parsed for every tile of the same page. Already planned for Phase 3. | 🟡 |
| 16 | `core::Source` | If another app truncates a memory-mapped file, reading it raises SIGBUS. Inherent to mmap; documented, and apps should hand the engine files they control. | 🟡 |

## What holds up

- No panics on 855 damaged or random inputs across zip, EPUB, HTML, CSS, text decoding, image headers, PDF loading and rendering.
- Indic syllable rules and search folding are covered by tests in all nine scripts.
- Memory stays flat on large files; positions survive re-layout.

## Fixes and results

| # | Fix | Check |
|---|---|---|
| 1 | JNI books live in a registry of ids, each behind its own lock; `close` removes the id. | test: 8 threads × 50 calls on one book; calls after `close`, a double `close` and a made-up id all return errors |
| 2 | Strings cross JNI as UTF-16 (`GetStringRegion`, `NewString`). | builds for every format switch; Android build in CI |
| 3 | CSS capture starts only for a `<style>` element that has content. | test `self_closing_style_does_not_swallow_text` |
| 4 | A carried page break is merged into the next block's style. | test `empty_page_break_element_carries_to_next_block` |
| 5 | Selector data kept in a parallel stack (no cloning); nesting capped at 256; no matching work without a stylesheet. | 20,000 nested divs: 8.7 s → under 0.5 s (robustness test) |
| 6 | Text files with one paragraph per line are read line by line. | 25,000-line file: 25,000 paragraphs, first page in 0.46 ms (was 786 ms) |
| 7 | Prefix sums for widths; binary search for the first break. | single 200,000-word paragraph: 786 ms → 250 ms (now dominated by shaping) |
| 8 | LRU keeps a tick-ordered index: O(log n) per operation; oversized values are returned but never stored. | tests, incl. 100,000 inserts within budget |
| 9 | Calls into the PDF interpreter are wrapped; a panic becomes `Error::Malformed`. | robustness test |
| 10 | Every zip offset and size goes through checked arithmetic; stored entries obey the 256 MB limit. | robustness test |
| 11 | Image decode limit lowered to 16 megapixels (64 MB while decoding). | |
| 12 | DRM check reads `encryption.xml` properly: only IDPF/Adobe font obfuscation of font files is allowed; `rights.xml` means DRM. | test `font_obfuscation_is_not_drm` |
| 13 | Shaping plans cached per font and script. | no measurable change on the 1,072-page text (noise ±15%); kept because it removes repeated work |
| 14 | Image fit uses at least 1 px of page height. | |

Still open, by design:
- #15 (render cache per tile) is Phase 3 work.
- #16 (SIGBUS on a truncated mapped file) is documented on `Source::open`.
- A single paragraph of hundreds of thousands of words is shaped as a whole before its first line is shown (250 ms for 200,000 words). Real books split text into paragraphs; revisit if a real file shows the problem.

The robustness test now runs in CI in a release build on every push.
