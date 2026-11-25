//! JNI bridge for Android.
//!
//! Kotlin side (in the app, `package dev.akkhor`):
//!
//! ```kotlin
//! object Native {
//!     init { System.loadLibrary("akkhor") }
//!     external fun registerFont(script: String, path: String)        // "Latn", "Beng", "Deva", ...
//!     external fun openFd(fd: Int, name: String?): Long              // 0 on failure
//!     external fun openPath(path: String): Long
//!     external fun close(handle: Long)
//!     external fun format(handle: Long): Int                         // 0 PDF, 1 EPUB, 2 TXT
//!     external fun count(handle: Long): Int                          // PDF pages, EPUB chapters
//!     external fun pageWidth(handle: Long, page: Int): Float         // PDF points
//!     external fun pageHeight(handle: Long, page: Int): Float
//!     external fun renderPdfTile(handle: Long, page: Int, scale: Float,
//!                                x: Int, y: Int, bitmap: Bitmap, theme: Int): Int
//!     external fun layoutChapter(handle: Long, chapter: Int, width: Int, height: Int,
//!                                fontSize: Float): Int               // page count, < 0 on error
//!     external fun renderFlowPage(handle: Long, page: Int, bitmap: Bitmap, theme: Int): Int
//!     // Positions are Longs: (block shl 32) or byteOffset. All snap to syllables.
//!     external fun hitTest(handle: Long, page: Int, x: Float, y: Float): Long   // -1 if none
//!     external fun wordAt(handle: Long, pos: Long): LongArray               // [start, end]
//!     external fun selectionRects(handle: Long, page: Int, start: Long, end: Long): FloatArray
//!     external fun textBetween(handle: Long, start: Long, end: Long): String
//!     external fun search(handle: Long, chapter: Int, query: String): LongArray // [start, end, ...]
//!     // PDF text (copy, search, selection, read-aloud). Positions are Longs:
//!     // (page shl 32) or byteOffset, snapped to syllables; boxes in PDF points.
//!     external fun pdfPageText(handle: Long, page: Int): String?
//!     external fun pdfHitTest(handle: Long, page: Int, x: Float, y: Float): Long // -1 if none
//!     external fun pdfWordAt(handle: Long, pos: Long): LongArray               // [start, end]
//!     external fun pdfSelectionRects(handle: Long, start: Long, end: Long): FloatArray
//!     external fun pdfTextBetween(handle: Long, start: Long, end: Long): String?
//!     external fun pdfSearch(handle: Long, page: Int, query: String): LongArray // one page
//!     external fun lastError(): String?
//! }
//! ```
//!
//! Rendering writes straight into the `Bitmap`'s memory (ARGB_8888), with no
//! intermediate copy. Every entry point catches panics so a bad file can
//! never crash the app; failures return an error code and `lastError()`
//! explains it.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use akkhor::{Book, Format, Source, Theme};
use jni_sys::{JNIEnv, jclass, jfloat, jint, jlong, jobject, jstring};

#[cfg(feature = "flow")]
use akkhor::flow::{FontSet, GlyphCache, LayoutOptions, Page, Script, fonts::FontData};

static FONTS: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_error(msg: impl ToString) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(msg.to_string()));
}

struct Handle {
    book: Book,
    #[cfg(feature = "flow")]
    flow: Option<FlowState>,
}

#[cfg(feature = "flow")]
struct FlowState {
    fonts: FontSet,
    glyphs: GlyphCache,
    pages: Vec<Page>,
    width: u32,
    height: u32,
    /// EPUB chapter the pages belong to (for resolving its images).
    chapter: usize,
    /// The laid-out chapter, for hit-testing, selection and copy.
    doc: akkhor::flow::Document,
}

#[cfg(feature = "flow")]
fn script_from_code(code: &str) -> Option<Script> {
    Some(match code {
        "Latn" => Script::Latin,
        "Beng" => Script::Bengali,
        "Deva" => Script::Devanagari,
        "Guru" => Script::Gurmukhi,
        "Gujr" => Script::Gujarati,
        "Orya" => Script::Oriya,
        "Taml" => Script::Tamil,
        "Telu" => Script::Telugu,
        "Knda" => Script::Kannada,
        "Mlym" => Script::Malayalam,
        _ => return None,
    })
}

#[cfg(feature = "flow")]
fn font_set() -> FontSet {
    let mut set = FontSet::new();
    let registered = FONTS.lock().map(|f| f.clone()).unwrap_or_default();
    for (code, path) in registered {
        if let Some(script) = script_from_code(&code) {
            let scripts: &[Script] = if script == Script::Latin {
                &[Script::Latin, Script::Common, Script::Other]
            } else {
                std::slice::from_ref(&script)
            };
            set.add(scripts, FontData::Path(path));
        }
    }
    set
}

fn theme(code: jint) -> Theme {
    match code {
        1 => Theme::SEPIA,
        2 => Theme::DUSK,
        3 => Theme::NIGHT,
        _ => Theme::PAPER,
    }
}

/// Runs `f`, turning errors and panics into `fallback`.
fn guard<T>(fallback: T, f: impl FnOnce() -> Result<T, String>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            set_error(e);
            fallback
        }
        Err(_) => {
            set_error("internal error (panic)");
            fallback
        }
    }
}

/// Reads a Java string as UTF-16 (JNI's "modified UTF-8" would mangle
/// characters outside the Basic Multilingual Plane).
unsafe fn jstr(env: *mut JNIEnv, s: jstring) -> Option<String> {
    if s.is_null() {
        return None;
    }
    unsafe {
        let len = (**env).GetStringLength?(env, s);
        let mut units = vec![0u16; len.max(0) as usize];
        (**env).GetStringRegion?(env, s, 0, len, units.as_mut_ptr());
        Some(String::from_utf16_lossy(&units))
    }
}

/// Makes a Java string from UTF-16, which is valid for every character.
unsafe fn new_jstring(env: *mut JNIEnv, s: &str) -> Result<jstring, String> {
    let units: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let new = (**env).NewString.ok_or("JNI")?;
        let out = new(env, units.as_ptr(), units.len() as jint);
        if out.is_null() {
            return Err("out of memory".into());
        }
        Ok(out)
    }
}

type Shared = Arc<Mutex<Handle>>;

/// Open books by id. Ids are never reused, so a stale or doubled `close`
/// and a call after `close` are harmless errors, and each book has its own
/// lock, so a render thread and the UI thread can call in at the same time.
static HANDLES: Mutex<BTreeMap<jlong, Shared>> = Mutex::new(BTreeMap::new());
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn registry() -> MutexGuard<'static, BTreeMap<jlong, Shared>> {
    HANDLES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs `f` on the open book `h`, holding that book's lock.
fn with_handle<T>(h: jlong, f: impl FnOnce(&mut Handle) -> Result<T, String>) -> Result<T, String> {
    let shared = registry()
        .get(&h)
        .cloned()
        .ok_or("closed or invalid handle")?;
    // A panic in an earlier call poisons the lock; the data is still
    // consistent (every call writes whole values), so carry on.
    let mut lock = shared.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut lock)
}

/// Runs `f` on the laid-out chapter of `h`.
#[cfg(feature = "flow")]
fn with_flow<T>(
    h: jlong,
    f: impl FnOnce(&mut FlowState) -> Result<T, String>,
) -> Result<T, String> {
    with_handle(h, |h| {
        f(h.flow.as_mut().ok_or("call layoutChapter first")?)
    })
}

fn into_handle(book: Book) -> jlong {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let handle = Handle {
        book,
        #[cfg(feature = "flow")]
        flow: None,
    };
    registry().insert(id, Arc::new(Mutex::new(handle)));
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_registerFont(
    env: *mut JNIEnv,
    _: jclass,
    script: jstring,
    path: jstring,
) {
    guard((), || {
        let script = unsafe { jstr(env, script) }.ok_or("script is null")?;
        let path = unsafe { jstr(env, path) }.ok_or("path is null")?;
        // Also a reference for PDF text recovery: PDFs embed stripped copies
        // of the same fonts. Memory-mapped, read only when a PDF needs it.
        #[cfg(feature = "pdf")]
        if let Ok(src) = Source::open(&path) {
            akkhor::pdf::register_reference_font(src.shared());
        }
        FONTS
            .lock()
            .map_err(|_| "font registry poisoned")?
            .push((script, PathBuf::from(path)));
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_openPath(
    env: *mut JNIEnv,
    _: jclass,
    path: jstring,
) -> jlong {
    guard(0, || {
        let path = unsafe { jstr(env, path) }.ok_or("path is null")?;
        let book = akkhor::open(&path).map_err(|e| e.to_string())?;
        Ok(into_handle(book))
    })
}

/// Opens a file descriptor, e.g. from `ContentResolver.openFileDescriptor`.
/// The descriptor is duplicated, so the caller may close its own copy.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_openFd(
    env: *mut JNIEnv,
    _: jclass,
    fd: jint,
    name: jstring,
) -> jlong {
    guard(0, || {
        #[cfg(unix)]
        {
            use std::os::fd::BorrowedFd;
            let name = unsafe { jstr(env, name) };
            // SAFETY: the caller guarantees `fd` is open for this call; we
            // duplicate it before use and never close the original.
            let owned = unsafe { BorrowedFd::borrow_raw(fd) }
                .try_clone_to_owned()
                .map_err(|e| e.to_string())?;
            let file = std::fs::File::from(owned);
            let source = Source::from_file(&file).map_err(|e| e.to_string())?;
            let book = akkhor::open_source(source, name.as_deref()).map_err(|e| e.to_string())?;
            Ok(into_handle(book))
        }
        #[cfg(not(unix))]
        {
            let _ = (env, fd, name);
            Err("file descriptors are only supported on Unix".into())
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_close(_: *mut JNIEnv, _: jclass, h: jlong) {
    // Remove it from the registry, then drop it outside the registry lock.
    // A call still running on another thread keeps it alive until it ends.
    let removed = registry().remove(&h);
    let _ = catch_unwind(AssertUnwindSafe(|| drop(removed)));
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_format(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jint {
    guard(-1, || {
        with_handle(h, |h| {
            Ok(match h.book.format() {
                Format::Pdf => 0,
                Format::Epub => 1,
                Format::Txt => 2,
                _ => 9,
            })
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_count(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jint {
    guard(-1, || {
        with_handle(h, |h| {
            Ok(match &h.book {
                #[cfg(feature = "pdf")]
                Book::Pdf(p) => p.page_count() as jint,
                #[cfg(feature = "epub")]
                Book::Epub(e) => e.chapter_count() as jint,
                #[cfg(feature = "txt")]
                Book::Text(_) => 1,
            })
        })
    })
}

#[cfg(feature = "pdf")]
fn pdf_size(h: jlong, page: jint) -> Result<(f32, f32), String> {
    with_handle(h, |h| match &h.book {
        Book::Pdf(p) => p.page_size(page as usize).map_err(|e| e.to_string()),
        #[allow(unreachable_patterns)]
        _ => Err("not a PDF".into()),
    })
}

#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pageWidth(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
) -> jfloat {
    guard(-1.0, || pdf_size(h, page).map(|s| s.0))
}

#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pageHeight(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
) -> jfloat {
    guard(-1.0, || pdf_size(h, page).map(|s| s.1))
}

/// Locked pixels of an Android `Bitmap`.
struct LockedBitmap {
    env: *mut JNIEnv,
    bitmap: jobject,
    pub width: u32,
    pub height: u32,
    pixels: *mut u8,
    stride: u32,
}

#[cfg(target_os = "android")]
mod bitmap {
    use super::*;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    pub struct AndroidBitmapInfo {
        pub width: u32,
        pub height: u32,
        pub stride: u32,
        pub format: i32,
        pub flags: u32,
    }
    pub const RGBA_8888: i32 = 1;

    #[link(name = "jnigraphics")]
    unsafe extern "C" {
        pub fn AndroidBitmap_getInfo(
            env: *mut JNIEnv,
            bmp: jobject,
            info: *mut AndroidBitmapInfo,
        ) -> i32;
        pub fn AndroidBitmap_lockPixels(
            env: *mut JNIEnv,
            bmp: jobject,
            addr: *mut *mut c_void,
        ) -> i32;
        pub fn AndroidBitmap_unlockPixels(env: *mut JNIEnv, bmp: jobject) -> i32;
    }
}

impl LockedBitmap {
    #[cfg(target_os = "android")]
    unsafe fn lock(env: *mut JNIEnv, bitmap: jobject) -> Result<Self, String> {
        use bitmap::*;
        unsafe {
            let mut info = AndroidBitmapInfo::default();
            if AndroidBitmap_getInfo(env, bitmap, &mut info) != 0 {
                return Err("could not read bitmap info".into());
            }
            if info.format != RGBA_8888 {
                return Err("bitmap must be ARGB_8888".into());
            }
            let mut addr = std::ptr::null_mut();
            if AndroidBitmap_lockPixels(env, bitmap, &mut addr) != 0 || addr.is_null() {
                return Err("could not lock bitmap".into());
            }
            Ok(Self {
                env,
                bitmap,
                width: info.width,
                height: info.height,
                pixels: addr as *mut u8,
                stride: info.stride,
            })
        }
    }

    #[cfg(not(target_os = "android"))]
    unsafe fn lock(_env: *mut JNIEnv, _bitmap: jobject) -> Result<Self, String> {
        Err("bitmaps are only available on Android".into())
    }

    /// Tightly packed RGBA rows. ARGB_8888 bitmaps are packed in practice;
    /// a padded bitmap is rejected rather than drawn wrongly.
    fn pixels(&mut self) -> Result<&mut [u8], String> {
        if self.stride != self.width * 4 {
            return Err(
                "bitmap rows are padded; use a bitmap whose width is a multiple of 16".into(),
            );
        }
        let len = self.width as usize * self.height as usize * 4;
        Ok(unsafe { std::slice::from_raw_parts_mut(self.pixels, len) })
    }
}

impl Drop for LockedBitmap {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        unsafe {
            bitmap::AndroidBitmap_unlockPixels(self.env, self.bitmap);
        }
        let _ = (self.env, self.bitmap);
    }
}

/// Renders the tile of `page` at `scale` whose top-left corner is (x, y) into
/// `bitmap`; the tile is the bitmap's size. Returns 0 on success.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_renderPdfTile(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    scale: jfloat,
    x: jint,
    y: jint,
    bitmap: jobject,
    theme_code: jint,
) -> jint {
    guard(-1, || {
        with_handle(h, |h| {
            #[allow(irrefutable_let_patterns)]
            let Book::Pdf(pdf) = &h.book else {
                return Err("not a PDF".into());
            };
            let mut bmp = unsafe { LockedBitmap::lock(env, bitmap) }?;
            let tile = akkhor::Tile::new(x.max(0) as u32, y.max(0) as u32, bmp.width, bmp.height);
            let out = bmp.pixels()?;
            pdf.render_tile(page as usize, scale, tile, &theme(theme_code), out)
                .map_err(|e| e.to_string())?;
            Ok(0)
        })
    })
}

/// Lays out one EPUB chapter (or a text file) for a screen of `width` x
/// `height` pixels. Returns the number of pages.
#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_layoutChapter(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    chapter: jint,
    width: jint,
    height: jint,
    font_size: jfloat,
) -> jint {
    guard(-1, || {
        with_handle(h, |h| {
            let _ = chapter; // unused in TXT-only builds
            let doc = match &h.book {
                #[cfg(feature = "epub")]
                Book::Epub(e) => e.chapter(chapter as usize).map_err(|e| e.to_string())?,
                #[cfg(feature = "txt")]
                Book::Text(t) => t.document.clone(),
                #[allow(unreachable_patterns)]
                _ => return Err("not a reflowable book".into()),
            };
            let flow = h.flow.get_or_insert_with(|| FlowState {
                fonts: font_set(),
                glyphs: GlyphCache::default(),
                pages: Vec::new(),
                width: 0,
                height: 0,
                chapter: 0,
                doc: Default::default(),
            });
            let opts = LayoutOptions {
                width: width as f32,
                height: height as f32,
                font_size,
                margin_x: width as f32 * 0.067,
                ..Default::default()
            };
            let images: Option<Box<dyn akkhor::flow::ImageProvider + '_>> = match &h.book {
                #[cfg(feature = "epub")]
                Book::Epub(e) => Some(Box::new(e.chapter_images(chapter as usize))),
                #[allow(unreachable_patterns)]
                _ => None,
            };
            flow.pages =
                akkhor::flow::paginate_with_images(&doc, &opts, &flow.fonts, images.as_deref());
            flow.chapter = chapter.max(0) as usize;
            flow.doc = doc;
            flow.width = width.max(0) as u32;
            flow.height = height.max(0) as u32;
            Ok(flow.pages.len() as jint)
        })
    })
}

#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_renderFlowPage(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    bitmap: jobject,
    theme_code: jint,
) -> jint {
    guard(-1, || {
        with_handle(h, |h| {
            let flow = h.flow.as_mut().ok_or("call layoutChapter first")?;
            let images: Option<Box<dyn akkhor::flow::ImageProvider + '_>> = match &h.book {
                #[cfg(feature = "epub")]
                Book::Epub(e) => Some(Box::new(e.chapter_images(flow.chapter))),
                #[allow(unreachable_patterns)]
                _ => None,
            };
            let p = flow.pages.get(page as usize).ok_or("page out of range")?;
            let mut bmp = unsafe { LockedBitmap::lock(env, bitmap) }?;
            if bmp.width != flow.width || bmp.height != flow.height {
                return Err("bitmap size differs from the laid-out page size".into());
            }
            let (w, hgt) = (bmp.width, bmp.height);
            let out = bmp.pixels()?;
            akkhor::flow::render_page(
                p,
                &flow.fonts,
                images.as_deref(),
                &mut flow.glyphs,
                &theme(theme_code),
                w,
                hgt,
                out,
            )
            .map_err(|e| e.to_string())?;
            Ok(0)
        })
    })
}

/// A position packed into one `Long`: block in the high 32 bits, byte
/// offset in the low 32 bits. Positions compare like the numbers.
#[cfg(feature = "flow")]
fn pack(p: akkhor::flow::Position) -> jlong {
    ((p.block as i64) << 32) | (p.offset as u32 as i64)
}

#[cfg(feature = "flow")]
fn unpack(v: jlong) -> akkhor::flow::Position {
    akkhor::flow::Position::new((v >> 32) as usize, (v & 0xFFFF_FFFF) as usize)
}

#[cfg(any(feature = "flow", feature = "pdf"))]
unsafe fn long_array(env: *mut JNIEnv, values: &[jlong]) -> Result<jni_sys::jlongArray, String> {
    unsafe {
        let new = (**env).NewLongArray.ok_or("JNI")?;
        let set = (**env).SetLongArrayRegion.ok_or("JNI")?;
        let arr = new(env, values.len() as jint);
        if arr.is_null() {
            return Err("out of memory".into());
        }
        set(env, arr, 0, values.len() as jint, values.as_ptr());
        Ok(arr)
    }
}

/// The position under (x, y) on a laid-out page, snapped to a syllable
/// boundary; -1 if the page is empty.
#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_hitTest(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    x: jfloat,
    y: jfloat,
) -> jlong {
    guard(-1, || {
        with_flow(h, |flow| {
            let p = flow.pages.get(page as usize).ok_or("page out of range")?;
            Ok(akkhor::flow::hit_test(&flow.doc, p, x, y).map_or(-1, pack))
        })
    })
}

/// The word around a position (long-press): `[start, end]`.
#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_wordAt(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    pos: jlong,
) -> jni_sys::jlongArray {
    guard(std::ptr::null_mut(), || {
        let (s, e) = with_flow(h, |flow| Ok(akkhor::flow::word_at(&flow.doc, unpack(pos))))?;
        unsafe { long_array(env, &[pack(s), pack(e)]) }
    })
}

/// Highlight rectangles on a page: `[x, y, width, height, ...]`.
#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_selectionRects(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    start: jlong,
    end: jlong,
) -> jni_sys::jfloatArray {
    guard(std::ptr::null_mut(), || {
        let rects = with_flow(h, |flow| {
            let p = flow.pages.get(page as usize).ok_or("page out of range")?;
            Ok(akkhor::flow::selection_rects(p, unpack(start), unpack(end)))
        })?;
        let flat: Vec<jfloat> = rects
            .iter()
            .flat_map(|r| [r.x, r.y, r.width, r.height])
            .collect();
        unsafe {
            let new = (**env).NewFloatArray.ok_or("JNI")?;
            let set = (**env).SetFloatArrayRegion.ok_or("JNI")?;
            let arr = new(env, flat.len() as jint);
            if arr.is_null() {
                return Err("out of memory".into());
            }
            set(env, arr, 0, flat.len() as jint, flat.as_ptr());
            Ok(arr)
        }
    })
}

/// The text between two positions of the laid-out chapter (copy).
#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_textBetween(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    start: jlong,
    end: jlong,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let text = with_flow(h, |flow| {
            Ok(akkhor::flow::text_between(
                &flow.doc,
                unpack(start),
                unpack(end),
            ))
        })?;
        unsafe { new_jstring(env, &text) }
    })
}

/// Searches one chapter (EPUB) or the whole text (TXT): `[start, end, ...]`.
#[cfg(feature = "flow")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_search(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    chapter: jint,
    query: jstring,
) -> jni_sys::jlongArray {
    guard(std::ptr::null_mut(), || {
        let _ = chapter; // unused in TXT-only builds
        let query = unsafe { jstr(env, query) }.ok_or("query is null")?;
        let doc = with_handle(h, |hd| match &hd.book {
            #[cfg(feature = "epub")]
            Book::Epub(e) => e.chapter(chapter as usize).map_err(|e| e.to_string()),
            #[cfg(feature = "txt")]
            Book::Text(t) => Ok(t.document.clone()),
            #[allow(unreachable_patterns)]
            _ => Err("not a reflowable book".into()),
        })?;
        let hits: Vec<jlong> = akkhor::flow::find_all(&doc, &query)
            .iter()
            .flat_map(|m| [pack(m.start), pack(m.end)])
            .collect();
        unsafe { long_array(env, &hits) }
    })
}

/// Runs `f` on a PDF page's text (cached for the last few pages).
#[cfg(feature = "pdf")]
fn with_pdf_text<T>(
    h: jlong,
    page: usize,
    f: impl FnOnce(&akkhor::pdf::PageText) -> Result<T, String>,
) -> Result<T, String> {
    let text = with_handle(h, |hd| match &hd.book {
        Book::Pdf(p) => p.page_text_cached(page).map_err(|e| e.to_string()),
        #[allow(unreachable_patterns)]
        _ => Err("not a PDF".into()),
    })?;
    f(&text)
}

#[cfg(feature = "pdf")]
fn pdf_pos(page: usize, offset: usize) -> jlong {
    ((page as i64) << 32) | (offset as u32 as i64)
}

#[cfg(feature = "pdf")]
fn pdf_unpos(v: jlong) -> (usize, usize) {
    ((v >> 32) as usize, (v & 0xFFFF_FFFF) as usize)
}

/// A PDF page's text, repaired (Indic conjuncts, reph, vowel order, Bijoy).
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfPageText(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let text = with_pdf_text(h, page.max(0) as usize, |t| Ok(t.text.clone()))?;
        unsafe { new_jstring(env, &text) }
    })
}

/// The text position under page point (x, y), in PDF points from the top
/// left; -1 if the page has no text.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfHitTest(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    x: jfloat,
    y: jfloat,
) -> jlong {
    guard(-1, || {
        let page = page.max(0) as usize;
        with_pdf_text(h, page, |t| {
            Ok(t.offset_at(x as f64, y as f64)
                .map_or(-1, |o| pdf_pos(page, o)))
        })
    })
}

/// The word around a PDF position (long-press): `[start, end]`.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfWordAt(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    pos: jlong,
) -> jni_sys::jlongArray {
    guard(std::ptr::null_mut(), || {
        let (page, offset) = pdf_unpos(pos);
        let r = with_pdf_text(h, page, |t| Ok(t.word_at(offset)))?;
        unsafe { long_array(env, &[pdf_pos(page, r.start), pdf_pos(page, r.end)]) }
    })
}

/// Highlight boxes for a PDF selection on one page: `[x, y, width, height,
/// ...]` in PDF points. `end` on a later page selects to the page's end.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfSelectionRects(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    start: jlong,
    end: jlong,
) -> jni_sys::jfloatArray {
    guard(std::ptr::null_mut(), || {
        let ((page, a), (end_page, b)) = (pdf_unpos(start), pdf_unpos(end));
        let rects = with_pdf_text(h, page, |t| {
            let b = if end_page > page {
                t.text.len()
            } else {
                b.min(t.text.len())
            };
            Ok(t.rects(a.min(b)..b))
        })?;
        let flat: Vec<jfloat> = rects
            .iter()
            .flat_map(|r| {
                [
                    r.x0 as f32,
                    r.y0 as f32,
                    r.width() as f32,
                    r.height() as f32,
                ]
            })
            .collect();
        unsafe {
            let new = (**env).NewFloatArray.ok_or("JNI")?;
            let set = (**env).SetFloatArrayRegion.ok_or("JNI")?;
            let arr = new(env, flat.len() as jint);
            if arr.is_null() {
                return Err("out of memory".into());
            }
            set(env, arr, 0, flat.len() as jint, flat.as_ptr());
            Ok(arr)
        }
    })
}

/// The text between two PDF positions (copy), across pages if needed.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfTextBetween(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    start: jlong,
    end: jlong,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let ((p0, a), (p1, b)) = (pdf_unpos(start), pdf_unpos(end));
        if p1 < p0 || p1 - p0 > 500 {
            return Err("bad range".into());
        }
        let mut out = String::new();
        for page in p0..=p1 {
            with_pdf_text(h, page, |t| {
                let from = if page == p0 { a.min(t.text.len()) } else { 0 };
                let to = if page == p1 {
                    b.min(t.text.len())
                } else {
                    t.text.len()
                };
                if from < to && t.text.is_char_boundary(from) && t.text.is_char_boundary(to) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&t.text[from..to]);
                }
                Ok(())
            })?;
        }
        unsafe { new_jstring(env, &out) }
    })
}

/// Searches one PDF page: `[start, end, ...]` positions. The app walks the
/// pages itself and stops when it has enough.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfSearch(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    query: jstring,
) -> jni_sys::jlongArray {
    guard(std::ptr::null_mut(), || {
        let query = unsafe { jstr(env, query) }.ok_or("query is null")?;
        let page = page.max(0) as usize;
        // Not through the page cache: a search sweep would evict the pages
        // the reader is looking at.
        let text = with_handle(h, |hd| match &hd.book {
            Book::Pdf(p) => p.page_text(page).map_err(|e| e.to_string()),
            #[allow(unreachable_patterns)]
            _ => Err("not a PDF".into()),
        })?;
        let hits: Vec<jlong> = text
            .find(&query)
            .into_iter()
            .flat_map(|r| [pdf_pos(page, r.start), pdf_pos(page, r.end)])
            .collect();
        unsafe { long_array(env, &hits) }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_lastError(
    env: *mut JNIEnv,
    _: jclass,
) -> jstring {
    match LAST_ERROR.with(|e| e.borrow().clone()) {
        Some(m) => unsafe { new_jstring(env, &m) }.unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn handles_are_safe_across_threads_and_after_close() {
        let id = into_handle(akkhor::open(fixture("small.pdf")).unwrap());
        // Many threads using the same book at once.
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let n = with_handle(id, |h| Ok(h.book.format())).unwrap();
                        assert_eq!(n, Format::Pdf);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        unsafe { Java_dev_akkhor_Native_close(std::ptr::null_mut(), std::ptr::null_mut(), id) };
        assert!(
            with_handle(id, |_| Ok(())).is_err(),
            "use after close is an error"
        );
        // A second close and a made-up id are harmless.
        unsafe { Java_dev_akkhor_Native_close(std::ptr::null_mut(), std::ptr::null_mut(), id) };
        assert!(with_handle(123_456_789, |_| Ok(())).is_err());
    }

    #[cfg(feature = "pdf")]
    #[test]
    fn pdf_text_through_the_handle() {
        let path = format!(
            "{}/../../tests/corpus/pdf/bn.chrome.pdf",
            env!("CARGO_MANIFEST_DIR")
        );
        let font = format!(
            "{}/../../fonts/NotoSerifBengali-Regular.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        akkhor::pdf::register_reference_font(Source::open(font).unwrap().shared());
        let id = into_handle(akkhor::open(path).unwrap());
        let text = with_pdf_text(id, 0, |t| Ok(t.text.clone())).unwrap();
        assert!(text.contains("যুক্তাক্ষরের"), "{text}");
        let at = text.find("যুক্তাক্ষরের").unwrap();
        let word = with_pdf_text(id, 0, |t| Ok(t.word_at(at + 3))).unwrap();
        assert_eq!(&text[word], "যুক্তাক্ষরের");
        assert_eq!(pdf_unpos(pdf_pos(7, 1234)), (7, 1234));
        assert!(pdf_pos(1, 0) > pdf_pos(0, 99_999));
        assert!(with_pdf_text(id, 99, |_| Ok(())).is_err());
        unsafe { Java_dev_akkhor_Native_close(std::ptr::null_mut(), std::ptr::null_mut(), id) };
    }

    #[test]
    fn positions_pack_in_reading_order() {
        use akkhor::flow::Position;
        let a = pack(Position::new(3, 10));
        let b = pack(Position::new(3, 11));
        let c = pack(Position::new(4, 0));
        assert!(a < b && b < c);
        assert_eq!(unpack(c), Position::new(4, 0));
    }
}
