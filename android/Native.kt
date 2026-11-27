// The Akkhor engine's interface for Android apps: copy this file into the
// app as-is. Every call is listed in docs/api/android.md with its errors.
// CI checks each declaration against the engine (scripts/check_native.py)
// and calls them all from a JVM (android/test/NativeTest.java).
//
// Threads: any call may come from any thread. Calls on one open book run
// one at a time (a long render delays a hit test on the same book; stop it
// with cancelRendering); calls on different books run in parallel.
// Errors: calls return null, -1 or 0 as documented, and lastError() says
// why. A bad file never crashes the app.

package dev.akkhor

import android.graphics.Bitmap

object Native {
    init { System.loadLibrary("akkhor") }

    /** The interface version this file was written for. */
    const val API_VERSION = 1

    // --- engine --------------------------------------------------------

    /** The engine's interface version; refuse an engine that differs. */
    external fun apiVersion(): Int

    /** Why the last call on this thread failed. */
    external fun lastError(): String?

    /** A font for a script ("Latn", "Beng", "Deva", "Guru", "Gujr", "Orya",
     *  "Taml", "Telu", "Knda", "Mlym"); also used to read PDFs that embed
     *  stripped copies of it. Register the phone's fonts at start-up. */
    external fun registerFont(script: String, path: String)

    /** One of the 14 standard PDF fonts ("Helvetica", "Times-Roman",
     *  "Courier", ...) for PDFs that use it without embedding it. */
    external fun registerStandardFont(name: String, path: String)

    /** The app's own reading colours for theme `code` (16 to 63), ARGB.
     *  Render calls then take that code. Built in: 0 paper, 1 sepia,
     *  2 dusk, 3 night, 4 original (PDF colours untouched). 0 or -1. */
    external fun setThemeColors(code: Int, background: Int, text: Int): Int

    // --- books ---------------------------------------------------------

    /** Opens a file; a handle, 0 on failure. */
    external fun openPath(path: String): Long

    /** Opens a file descriptor (ContentResolver.openFileDescriptor). The
     *  descriptor is duplicated: the caller may close its own. */
    external fun openFd(fd: Int, name: String?): Long

    /** Closes a book; harmless twice. A call still running ends first. */
    external fun close(handle: Long)

    /** 0 PDF, 1 EPUB, 2 TXT; -1 on error. */
    external fun format(handle: Long): Int

    /** PDF pages, EPUB chapters, 1 for TXT. */
    external fun count(handle: Long): Int

    /** The book's description as JSON: {format, title, authors[],
     *  language, language_guessed, publisher, length, has_cover,
     *  reflowable, text_quality, needs_fonts, bijoy}. */
    external fun info(handle: Long): String?

    /** Draws the cover fitted into `bitmap` (top left, rest cleared).
     *  Returns (width shl 32) or height of what was drawn; 0 when the book
     *  has no cover (draw a placeholder); -1 on error. */
    external fun renderCover(handle: Long, bitmap: Bitmap): Long

    /** Table of contents: titles, and [level, page or chapter, ...]. */
    external fun outlineTitles(handle: Long): Array<String>?
    external fun outlineLevelsAndPages(handle: Long): IntArray?

    /** Frees caches; call from onTrimMemory. Bytes freed. */
    external fun trimMemory(handle: Long): Int

    // --- PDF page view -----------------------------------------------

    /** Page size in PDF points. */
    external fun pageWidth(handle: Long, page: Int): Float
    external fun pageHeight(handle: Long, page: Int): Float

    /** Draws the tile of `page` at `scale` (pixels per point) whose top
     *  left is (x, y), the bitmap's size. 0 on success. Thumbnails: scale
     *  = side / max(pageWidth, pageHeight). */
    external fun renderPdfTile(handle: Long, page: Int, scale: Float,
                               x: Int, y: Int, bitmap: Bitmap, theme: Int): Int

    /** Several tiles of one page, sharing its fonts and images; returns
     *  how many were drawn (cancelRendering stops before the next). */
    external fun renderPdfTiles(handle: Long, page: Int, scale: Float,
                                bitmaps: Array<Bitmap>, xs: IntArray, ys: IntArray,
                                theme: Int): Int
    external fun cancelRendering(handle: Long)

    /** Page text, positions as Longs: (page shl 32) or byte offset, boxes
     *  in PDF points, snapped to syllables. */
    external fun pdfPageText(handle: Long, page: Int): String?
    external fun pdfHitTest(handle: Long, page: Int, x: Float, y: Float): Long
    external fun pdfWordAt(handle: Long, pos: Long): LongArray
    external fun pdfSelectionRects(handle: Long, start: Long, end: Long): FloatArray
    external fun pdfTextBetween(handle: Long, start: Long, end: Long): String?
    external fun pdfSearch(handle: Long, page: Int, query: String): LongArray

    /** Chosen pages (0-based, in this order) saved as a new PDF at [path]
     *  (share a chapter or a few pages). Each page keeps everything it uses,
     *  so its text stays selectable; links, form fields' links to other
     *  pages and the outline are left out. Returns the bytes written, or -1:
     *  encrypted PDFs are refused (lastError says so), as are pages out of
     *  range. Write to the app's cache, then share the file. */
    external fun pdfWritePages(handle: Long, pages: IntArray, path: String): Long

    // --- reflowed text (EPUB, TXT, PDF reflow) -------------------------

    /** Reflowed chapters: EPUB chapters, 1 for TXT, a PDF's reflow
     *  chapters. */
    external fun chapterCount(handle: Long): Int

    /** Lays out chapter `chapter` for a screen of width × height pixels
     *  at `fontSize` pixels; the number of screens, < 0 on error. */
    external fun layoutChapter(handle: Long, chapter: Int, width: Int, height: Int,
                               fontSize: Float): Int
    external fun renderFlowPage(handle: Long, page: Int, bitmap: Bitmap, theme: Int): Int

    /** Positions in the laid-out chapter as Longs: (block shl 32) or byte
     *  offset, snapped to syllables. */
    external fun hitTest(handle: Long, page: Int, x: Float, y: Float): Long
    external fun wordAt(handle: Long, pos: Long): LongArray
    external fun selectionRects(handle: Long, page: Int, start: Long, end: Long): FloatArray
    external fun textBetween(handle: Long, start: Long, end: Long): String?
    external fun search(handle: Long, chapter: Int, query: String): LongArray

    /** PDF reflow chapters and the same place in the other view, as
     *  packed positions (the locator calls below do the same by string). */
    external fun reflowChapterCount(handle: Long): Int
    external fun reflowChapterOfPage(handle: Long, page: Int): Int
    external fun pdfToReflow(handle: Long, chapter: Int, pdfPos: Long): Long
    external fun reflowToPdf(handle: Long, chapter: Int, pos: Long): Long

    // --- locators: the places the app stores -----------------------

    /** Locators are strings, "akkhor:1/c3/b12/o340" (reflowed text) or
     *  "akkhor:1/p7/o0" (a PDF page); they survive font size, screen and
     *  theme changes. The same place inside the book (past the end
     *  becomes the end). */
    external fun clampLocator(handle: Long, locator: String): String?

    /** A PDF place in the other view (page place <-> reflow place). */
    external fun convertLocator(handle: Long, locator: String): String?

    /** How far through the book a place is, 0 to 1; -1 on error. */
    external fun progressOf(handle: Long, locator: String): Double

    /** The reflowed place at `progress` (0 to 1): the scrubber. */
    external fun locatorAtProgress(handle: Long, progress: Double): String?

    /** Where a screen of the laid-out chapter starts (save this). */
    external fun locatorOfScreen(handle: Long, screen: Int): String?

    /** The screen of the laid-out chapter showing a place; -1 when the
     *  place is in another chapter (lay that one out first). */
    external fun screenOfLocator(handle: Long, locator: String): Int

    // --- ranges: highlights, notes, copy, quote cards ---------------

    /** The text between two places, across chapters or pages. */
    external fun textOfRange(handle: Long, start: String, end: String): String?

    /** Boxes [x, y, w, h, ...] of a range on screen `screen` of the
     *  laid-out chapter (pixels), or on its PDF page (points). */
    external fun rectsOfRange(handle: Long, screen: Int, start: String, end: String): FloatArray?

    // --- read aloud ------------------------------------------------

    /** Up to `count` sentences from a place on, JSON [{start, end, text}].
     *  Android's TextToSpeech reports UTF-16 ranges inside `text`. */
    external fun sentences(handle: Long, from: String, count: Int): String?

    // --- whole-book search -------------------------------------------

    /** Starts a search; its id, 0 on error. */
    external fun searchStart(handle: Long, query: String): Long

    /** Up to `max` more hits, JSON {hits: [{start, end, snippet,
     *  match_start, match_end, at, in_picture}], done, searched}. The
     *  match is in UTF-16 units of the snippet. Null once done. */
    external fun searchNext(search: Long, max: Int): String?

    /** Stops a search; harmless after it is done. */
    external fun searchCancel(search: Long)

    // --- strings ---------------------------------------------------

    /** Engine offsets are UTF-8 bytes; Kotlin strings count UTF-16. */
    external fun utf16Offset(text: String, utf8: Int): Int
    external fun utf8Offset(text: String, utf16: Int): Int

    /** The place [utf16] units into [text], where [text] starts at the
     *  locator [start]: for read aloud, a sentence from [sentences] and the
     *  offset TextToSpeech's onRangeStart reports, so the word being read
     *  can be highlighted with [rectsOfRange]. Null for a bad locator. */
    external fun locatorInText(start: String, text: String, utf16: Int): String?
}
