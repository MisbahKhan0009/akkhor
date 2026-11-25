//! Phase 5 calls for the reader app: book info and covers, locators and
//! progress, whole-book search, ranges, sentences for read aloud and the
//! app's own theme colours. Structured results are JSON strings
//! (`docs/api/android.md` lists every call and its result).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use akkhor::search::BookSearch;
use akkhor::{Book, Locator, Reading, Rgba, Theme};
use jni_sys::{JNIEnv, jclass, jdouble, jfloat, jint, jlong, jobject, jstring};

use super::{Handle, LockedBitmap, guard, jstr, new_jstring, with_handle};

/// The interface version. It goes up when a call changes meaning or
/// signature; new calls alone do not change it.
pub const API_VERSION: jint = 1;

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_akkhor_Native_apiVersion(_: *mut JNIEnv, _: jclass) -> jint {
    API_VERSION
}

// --- themes ----------------------------------------------------------------

/// Colours the app defined, by theme code (16 to 63).
static THEMES: Mutex<BTreeMap<jint, Theme>> = Mutex::new(BTreeMap::new());

/// Theme codes the app may define.
pub const CUSTOM_THEMES: std::ops::RangeInclusive<jint> = 16..=63;

/// The app's colours for theme `code` (16 to 63), as ARGB ints
/// (`0xFFF7F3EA`); render calls then take that code. Alpha is ignored.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_akkhor_Native_setThemeColors(
    _: *mut JNIEnv,
    _: jclass,
    code: jint,
    background: jint,
    text: jint,
) -> jint {
    guard(-1, || {
        if !CUSTOM_THEMES.contains(&code) {
            return Err(format!("theme code must be 16 to 63, not {code}"));
        }
        let rgb = |c: jint| {
            let c = c as u32;
            Rgba::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
        };
        THEMES.lock().map_err(|_| "themes poisoned")?.insert(
            code,
            Theme {
                background: rgb(background),
                text: rgb(text),
            },
        );
        Ok(0)
    })
}

/// A theme the app defined, if `code` names one.
pub fn custom_theme(code: jint) -> Option<Theme> {
    THEMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&code)
        .copied()
}

// --- helpers ---------------------------------------------------------------

/// Runs `f` with the book and its reading state (made on first use).
fn with_reading<T>(
    h: jlong,
    f: impl FnOnce(&Book, &mut Reading) -> Result<T, String>,
) -> Result<T, String> {
    with_handle(h, |hd: &mut Handle| {
        let Handle { book, reading, .. } = hd;
        let reading = reading.get_or_insert_with(|| Reading::new(book));
        f(book, reading)
    })
}

unsafe fn locator(env: *mut JNIEnv, s: jstring) -> Result<Locator, String> {
    let s = unsafe { jstr(env, s) }.ok_or("locator is null")?;
    Locator::parse(&s).ok_or_else(|| format!("not a locator: {s}"))
}

unsafe fn string_or_null(env: *mut JNIEnv, r: Result<String, String>) -> Result<jstring, String> {
    unsafe { new_jstring(env, &r?) }
}

unsafe fn float_array(env: *mut JNIEnv, values: &[jfloat]) -> Result<jni_sys::jfloatArray, String> {
    unsafe {
        let new = (**env).NewFloatArray.ok_or("JNI")?;
        let set = (**env).SetFloatArrayRegion.ok_or("JNI")?;
        let arr = new(env, values.len() as jint);
        if arr.is_null() {
            return Err("out of memory".into());
        }
        set(env, arr, 0, values.len() as jint, values.as_ptr());
        Ok(arr)
    }
}

// --- 5.1 book info and covers ---------------------------------------------

/// The book's description as JSON (format, title, authors, language,
/// length, cover, reflowable, text quality, needs_fonts, Bijoy).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_info(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        string_or_null(env, with_handle(h, |hd| Ok(hd.book.info().to_json())))
    })
}

/// Draws the cover fitted into `bitmap` (top left; the rest cleared) and
/// returns its size as (width shl 32) or height; 0 when the book has no
/// cover (draw a placeholder), -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_renderCover(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    bitmap: jobject,
) -> jlong {
    guard(-1, || {
        let mut bmp = unsafe { LockedBitmap::lock(env, bitmap)? };
        let (bw, bh) = (bmp.width, bmp.height);
        let Some(cover) = with_handle(h, |hd| Ok(hd.book.cover(bw, bh)))? else {
            return Ok(0);
        };
        let out = bmp.pixels()?;
        out.fill(0);
        let (w, h) = (cover.width.min(bw) as usize, cover.height.min(bh) as usize);
        for y in 0..h {
            let src = &cover.rgba[y * cover.width as usize * 4..][..w * 4];
            let dst = &mut out[y * bw as usize * 4..][..w * 4];
            // Android bitmaps hold premultiplied colour.
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let a = u16::from(s[3]);
                for k in 0..3 {
                    d[k] = ((u16::from(s[k]) * a + 127) / 255) as u8;
                }
                d[3] = s[3];
            }
        }
        Ok(((w as jlong) << 32) | h as jlong)
    })
}

// --- 5.2 locators and progress ------------------------------------------

/// Reflowed chapters: EPUB chapters, 1 for a text file, a PDF's reflow
/// chapters (the `chapter` of layoutChapter).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_akkhor_Native_chapterCount(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jint {
    guard(-1, || with_reading(h, |_, r| Ok(r.chapter_count() as jint)))
}

/// The same place inside the book (past the end becomes the end).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_clampLocator(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    loc: jstring,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let loc = locator(env, loc)?;
        let r = with_reading(h, |b, r| r.clamp(b, loc).map_err(|e| e.to_string()));
        string_or_null(env, r.map(|l| l.to_string()))
    })
}

/// A PDF place in the other view (page place to reflow place and back).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_convertLocator(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    loc: jstring,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let loc = locator(env, loc)?;
        let r = with_reading(h, |b, r| r.convert(b, loc).map_err(|e| e.to_string()));
        string_or_null(env, r.map(|l| l.to_string()))
    })
}

/// How far through the book a place is, 0 to 1; -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_progressOf(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    loc: jstring,
) -> jdouble {
    guard(-1.0, || {
        let loc = unsafe { locator(env, loc)? };
        with_reading(h, |b, r| r.progress(b, loc).map_err(|e| e.to_string()))
    })
}

/// The reflowed place at `progress` (0 to 1), for the scrubber.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_locatorAtProgress(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    progress: jdouble,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let r = with_reading(h, |b, r| {
            r.at_progress(b, progress).map_err(|e| e.to_string())
        });
        string_or_null(env, r.map(|l| l.to_string()))
    })
}

/// Where screen `screen` of the laid-out chapter starts.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_locatorOfScreen(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    screen: jint,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let r = with_handle(h, |hd| {
            let flow = hd.flow.as_ref().ok_or("call layoutChapter first")?;
            let page = flow
                .pages
                .get(screen.max(0) as usize)
                .ok_or("screen out of range")?;
            let (block, offset) = page.start().unwrap_or((0, 0));
            Ok(Locator::Flow {
                chapter: flow.chapter,
                block,
                offset,
            }
            .to_string())
        });
        string_or_null(env, r)
    })
}

/// The screen of the laid-out chapter that shows a reflow place; -1 when
/// the place is in another chapter (lay that one out first).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_screenOfLocator(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    loc: jstring,
) -> jint {
    guard(-1, || {
        let loc = unsafe { locator(env, loc)? };
        with_handle(h, |hd| {
            let flow = hd.flow.as_ref().ok_or("call layoutChapter first")?;
            match loc {
                Locator::Flow {
                    chapter,
                    block,
                    offset,
                } if chapter == flow.chapter => {
                    Ok(akkhor::reading::screen_of(&flow.pages, block, offset) as jint)
                }
                _ => Ok(-1),
            }
        })
    })
}

// --- 5.4 ranges --------------------------------------------------------------

/// The text between two places (blocks joined by line breaks).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_textOfRange(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    start: jstring,
    end: jstring,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let (a, b) = (locator(env, start)?, locator(env, end)?);
        let r = with_reading(h, |book, r| {
            r.text_between(book, a, b).map_err(|e| e.to_string())
        });
        string_or_null(env, r)
    })
}

/// Boxes [x, y, width, height, ...] covering a range: on screen `screen`
/// of the laid-out chapter for reflow places (pixels), on its page for
/// PDF page places (points; `screen` is ignored). Snapped to syllables.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_rectsOfRange(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    screen: jint,
    start: jstring,
    end: jstring,
) -> jni_sys::jfloatArray {
    guard(std::ptr::null_mut(), || unsafe {
        let (a, b) = (locator(env, start)?, locator(env, end)?);
        let flat: Vec<jfloat> = with_handle(h, |hd| match (a, b) {
            (
                Locator::Flow {
                    chapter: c0,
                    block: b0,
                    offset: o0,
                },
                Locator::Flow {
                    chapter: c1,
                    block: b1,
                    offset: o1,
                },
            ) => {
                let flow = hd.flow.as_ref().ok_or("call layoutChapter first")?;
                let page = flow
                    .pages
                    .get(screen.max(0) as usize)
                    .ok_or("screen out of range")?;
                let ch = flow.chapter;
                // The part of the range inside this chapter.
                let from = if c0 < ch { (0, 0) } else { (b0, o0) };
                let to = if c1 > ch { (usize::MAX, 0) } else { (b1, o1) };
                if c0 > ch || c1 < ch {
                    return Ok(Vec::new());
                }
                let rects = akkhor::flow::selection_rects(
                    page,
                    akkhor::flow::Position::new(from.0, from.1),
                    akkhor::flow::Position::new(to.0, to.1),
                );
                Ok(rects
                    .iter()
                    .flat_map(|r| [r.x, r.y, r.width, r.height])
                    .collect())
            }
            #[cfg(feature = "pdf")]
            (
                Locator::Page { page, offset: o0 },
                Locator::Page {
                    page: p1,
                    offset: o1,
                },
            ) => {
                let Book::Pdf(pdf) = &hd.book else {
                    return Err("not a PDF".into());
                };
                if p1 < page {
                    return Ok(Vec::new());
                }
                let t = pdf.page_text_cached(page).map_err(|e| e.to_string())?;
                let end = if p1 > page {
                    t.text.len()
                } else {
                    o1.min(t.text.len())
                };
                Ok(t.rects(o0.min(end)..end)
                    .iter()
                    .flat_map(|r| {
                        [
                            r.x0 as f32,
                            r.y0 as f32,
                            r.width() as f32,
                            r.height() as f32,
                        ]
                    })
                    .collect())
            }
            _ => Err("both ends must be the same kind of place".into()),
        })?;
        float_array(env, &flat)
    })
}

// --- 5.5 read aloud ----------------------------------------------------------

/// Up to `count` sentences from a place on, as a JSON array of
/// {start, end, text}.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_sentences(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    from: jstring,
    count: jint,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let from = locator(env, from)?;
        let r = with_reading(h, |b, r| {
            r.sentences(b, from, count.clamp(1, 1000) as usize)
                .map(|s| akkhor::json::array(s.iter().map(|x| x.to_json())))
                .map_err(|e| e.to_string())
        });
        string_or_null(env, r)
    })
}

// --- 5.3 whole-book search ---------------------------------------------

/// A search in progress, and the book it searches.
type Running = std::sync::Arc<Mutex<(jlong, BookSearch)>>;

/// Searches in progress by id.
static SEARCHES: Mutex<BTreeMap<jlong, Running>> = Mutex::new(BTreeMap::new());
static NEXT_SEARCH: AtomicI64 = AtomicI64::new(1);

/// Starts a search of the whole book; returns its id (0 on error).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_searchStart(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    query: jstring,
) -> jlong {
    guard(0, || {
        let query = unsafe { jstr(env, query) }.ok_or("query is null")?;
        with_handle(h, |_| Ok(()))?;
        let id = NEXT_SEARCH.fetch_add(1, Ordering::Relaxed);
        SEARCHES.lock().map_err(|_| "searches poisoned")?.insert(
            id,
            std::sync::Arc::new(Mutex::new((h, BookSearch::new(&query)))),
        );
        Ok(id)
    })
}

/// Up to `max` more hits as JSON {hits: [{start, end, snippet,
/// match_start, match_end, at, in_picture}], done, searched}.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_searchNext(
    env: *mut JNIEnv,
    _: jclass,
    search: jlong,
    max: jint,
) -> jstring {
    guard(std::ptr::null_mut(), || unsafe {
        let running = SEARCHES
            .lock()
            .map_err(|_| "searches poisoned")?
            .get(&search)
            .cloned()
            .ok_or("no such search (finished or cancelled)")?;
        // The search's own lock, not the table's: other searches go on.
        let mut guard = running.lock().map_err(|_| "search poisoned")?;
        let (h, s) = &mut *guard;
        let batch = with_reading(*h, |b, r| Ok(s.next(b, r, max.clamp(1, 500) as usize)))?;
        if batch.done {
            SEARCHES
                .lock()
                .map_err(|_| "searches poisoned")?
                .remove(&search);
        }
        new_jstring(env, &batch.to_json())
    })
}

/// Stops a search and frees it. Harmless for a finished or unknown id.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_akkhor_Native_searchCancel(
    _: *mut JNIEnv,
    _: jclass,
    search: jlong,
) {
    guard((), || {
        SEARCHES
            .lock()
            .map_err(|_| "searches poisoned")?
            .remove(&search);
        Ok(())
    })
}

/// Searches of a closed book are dropped with it.
pub fn forget_searches(h: jlong) {
    if let Ok(mut s) = SEARCHES.lock() {
        s.retain(|_, r| r.lock().is_ok_and(|g| g.0 != h));
    }
}
