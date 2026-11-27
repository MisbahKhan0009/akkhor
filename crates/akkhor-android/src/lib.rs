//! JNI bridge for Android.
//!
//! The Kotlin side is [`android/Native.kt`](../../../android/Native.kt),
//! which the app copies; `docs/api/android.md` explains the calls. CI checks
//! that every declaration there matches a function here
//! (`scripts/check_native.py`) and calls them all from a JVM.
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
use jni_sys::{JNIEnv, jboolean, jclass, jfloat, jint, jlong, jobject, jstring};

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
    /// Chapters, progress and locators (Phase 5), made on first use.
    #[cfg(feature = "flow")]
    reading: Option<akkhor::Reading>,
}

#[cfg(feature = "flow")]
mod app;

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
    /// A PDF's reflow chapter, with its map back to the pages.
    #[cfg(feature = "pdf")]
    reflow: Option<(usize, akkhor::pdf::reflow::Reflow)>,
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

/// 0 paper, 1 sepia, 2 dusk, 3 night, 4 original (PDF colours untouched),
/// 16 to 63 the app's own (setThemeColors).
fn theme(code: jint) -> Theme {
    #[cfg(feature = "flow")]
    if let Some(t) = app::custom_theme(code) {
        return t;
    }
    match code {
        1 => Theme::SEPIA,
        2 => Theme::DUSK,
        3 => Theme::NIGHT,
        4 => Theme::ORIGINAL,
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

/// Per open book, a counter that `cancelRendering` bumps. Kept outside the
/// book's own lock, which a running render holds.
static GENERATIONS: Mutex<BTreeMap<jlong, Arc<std::sync::atomic::AtomicU64>>> =
    Mutex::new(BTreeMap::new());

#[cfg(any(feature = "pdf", feature = "cbz"))]
fn generation(h: jlong) -> Option<Arc<std::sync::atomic::AtomicU64>> {
    GENERATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&h)
        .cloned()
}

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
        #[cfg(feature = "flow")]
        reading: None,
    };
    registry().insert(id, Arc::new(Mutex::new(handle)));
    GENERATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, Arc::new(std::sync::atomic::AtomicU64::new(0)));
    id
}

/// Supplies a standard PDF font (by PostScript name) from a font file.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_registerStandardFont(
    env: *mut JNIEnv,
    _: jclass,
    name: jstring,
    path: jstring,
) {
    guard((), || {
        let name = unsafe { jstr(env, name) }.ok_or("name is null")?;
        let path = unsafe { jstr(env, path) }.ok_or("path is null")?;
        let src = akkhor::Source::open(&path).map_err(|e| e.to_string())?;
        akkhor::pdf::register_standard_font(&name, src.shared());
        Ok(())
    })
}

/// The app's cache folder. Tables the engine builds from registered fonts
/// (a few tens of milliseconds each) are kept there, keyed by the font's
/// bytes and the engine build. Only the first call counts.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_setCacheDir(
    env: *mut JNIEnv,
    _: jclass,
    path: jstring,
) -> jboolean {
    guard(0, || {
        let path = unsafe { jstr(env, path) }.ok_or("path is null")?;
        #[cfg(feature = "pdf")]
        return Ok(u8::from(akkhor::pdf::set_cache_dir(path)));
        #[cfg(not(feature = "pdf"))]
        {
            let _ = path;
            Ok(1)
        }
    })
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
    #[cfg(feature = "flow")]
    app::forget_searches(h);
    if let Some(g) = GENERATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&h)
    {
        // A render still running on another thread stops at its next tile.
        g.fetch_add(1, Ordering::Relaxed);
    }
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
                Format::Mobi => 3,
                Format::Fb2 => 4,
                Format::Cbz => 5,
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
                #[cfg(feature = "cbz")]
                Book::Cbz(c) => c.page_count() as jint,
                #[allow(unreachable_patterns)]
                book => book.flow().map_or(0, |f| f.chapter_count()) as jint,
            })
        })
    })
}

/// Size of a page of a page book (PDF points, comic pixels).
#[cfg(any(feature = "pdf", feature = "cbz"))]
fn page_size(h: jlong, page: jint) -> Result<(f32, f32), String> {
    with_handle(h, |h| match &h.book {
        #[cfg(feature = "pdf")]
        Book::Pdf(p) => p.page_size(page.max(0) as usize).map_err(|e| e.to_string()),
        #[cfg(feature = "cbz")]
        Book::Cbz(c) => c.page_size(page.max(0) as usize).map_err(|e| e.to_string()),
        #[allow(unreachable_patterns)]
        _ => Err("not a page book (PDF or comic)".into()),
    })
}

/// Draws `tile` of `page` of a page book into `out`.
#[cfg(any(feature = "pdf", feature = "cbz"))]
fn draw_tile(
    book: &Book,
    page: usize,
    scale: f32,
    tile: akkhor::Tile,
    theme: &akkhor::Theme,
    out: &mut [u8],
) -> Result<(), String> {
    match book {
        #[cfg(feature = "pdf")]
        Book::Pdf(pdf) => pdf
            .render_tile(page, scale, tile, theme, out)
            .map_err(|e| e.to_string()),
        #[cfg(feature = "cbz")]
        Book::Cbz(c) => {
            let r = c
                .render_tile(page, scale, tile, theme)
                .map_err(|e| e.to_string())?;
            let n = r.pixels.len().min(out.len());
            out[..n].copy_from_slice(&r.pixels[..n]);
            Ok(())
        }
        #[allow(unreachable_patterns)]
        _ => Err("not a page book (PDF or comic)".into()),
    }
}

#[cfg(any(feature = "pdf", feature = "cbz"))]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pageWidth(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
) -> jfloat {
    guard(-1.0, || page_size(h, page).map(|s| s.0))
}

#[cfg(any(feature = "pdf", feature = "cbz"))]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pageHeight(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
) -> jfloat {
    guard(-1.0, || page_size(h, page).map(|s| s.1))
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
/// `bitmap`; the tile is the bitmap's size. PDFs and comics. Returns 0 on
/// success.
#[cfg(any(feature = "pdf", feature = "cbz"))]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_renderPageTile(
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
            let mut bmp = unsafe { LockedBitmap::lock(env, bitmap) }?;
            let tile = akkhor::Tile::new(x.max(0) as u32, y.max(0) as u32, bmp.width, bmp.height);
            let out = bmp.pixels()?;
            draw_tile(
                &h.book,
                page.max(0) as usize,
                scale,
                tile,
                &theme(theme_code),
                out,
            )?;
            Ok(0)
        })
    })
}

/// Draws several tiles of one page, one per bitmap, at `xs[i]`, `ys[i]`
/// (each tile is its bitmap's size), sharing the page's parsed fonts and
/// images. Stops before the next tile once `cancelRendering` is called.
/// Returns the number of tiles drawn, or -1 on error.
#[cfg(any(feature = "pdf", feature = "cbz"))]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_renderPageTiles(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
    scale: jfloat,
    bitmaps: jni_sys::jobjectArray,
    xs: jni_sys::jintArray,
    ys: jni_sys::jintArray,
    theme_code: jint,
) -> jint {
    guard(-1, || {
        let generation = generation(h).ok_or("closed or invalid handle")?;
        let started = generation.load(Ordering::Relaxed);
        let (n, xs, ys) = unsafe {
            let len = (**env).GetArrayLength.ok_or("JNI")?;
            let n = len(env, bitmaps).max(0) as usize;
            if len(env, xs) as usize != n || len(env, ys) as usize != n {
                return Err("bitmaps, xs and ys differ in length".into());
            }
            let get = (**env).GetIntArrayRegion.ok_or("JNI")?;
            let (mut vx, mut vy) = (vec![0 as jint; n], vec![0 as jint; n]);
            get(env, xs, 0, n as jint, vx.as_mut_ptr());
            get(env, ys, 0, n as jint, vy.as_mut_ptr());
            (n, vx, vy)
        };
        with_handle(h, |hd| {
            let mut drawn = 0;
            for i in 0..n {
                if generation.load(Ordering::Relaxed) != started {
                    break;
                }
                let bitmap = unsafe {
                    let get = (**env).GetObjectArrayElement.ok_or("JNI")?;
                    get(env, bitmaps, i as jint)
                };
                let mut bmp = unsafe { LockedBitmap::lock(env, bitmap) }?;
                let tile = akkhor::Tile::new(
                    xs[i].max(0) as u32,
                    ys[i].max(0) as u32,
                    bmp.width,
                    bmp.height,
                );
                let out = bmp.pixels()?;
                draw_tile(
                    &hd.book,
                    page.max(0) as usize,
                    scale,
                    tile,
                    &theme(theme_code),
                    out,
                )?;
                drop(bmp);
                unsafe {
                    if let Some(del) = (**env).DeleteLocalRef {
                        del(env, bitmap);
                    }
                }
                drawn += 1;
            }
            Ok(drawn)
        })
    })
}

#[cfg(feature = "pdf")]
fn pdf_outline(h: jlong) -> Result<Vec<akkhor::pdf::OutlineItem>, String> {
    with_handle(h, |hd| match &hd.book {
        Book::Pdf(p) => Ok(p.outline()),
        #[allow(unreachable_patterns)]
        _ => Err("not a PDF".into()),
    })
}

/// The PDF's table of contents: the entries' titles, in order.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_outlineTitles(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jni_sys::jobjectArray {
    guard(std::ptr::null_mut(), || {
        let items = pdf_outline(h)?;
        unsafe {
            let find = (**env).FindClass.ok_or("JNI")?;
            let class = find(env, c"java/lang/String".as_ptr());
            if class.is_null() {
                return Err("no String class".into());
            }
            let new = (**env).NewObjectArray.ok_or("JNI")?;
            let set = (**env).SetObjectArrayElement.ok_or("JNI")?;
            let del = (**env).DeleteLocalRef.ok_or("JNI")?;
            let arr = new(env, items.len() as jint, class, std::ptr::null_mut());
            if arr.is_null() {
                return Err("out of memory".into());
            }
            for (i, item) in items.iter().enumerate() {
                let s = new_jstring(env, &item.title)?;
                set(env, arr, i as jint, s);
                del(env, s);
            }
            Ok(arr)
        }
    })
}

/// The same entries' nesting levels and pages: `[level, page, ...]`, page
/// -1 when an entry points nowhere in this document.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_outlineLevelsAndPages(
    env: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jni_sys::jintArray {
    guard(std::ptr::null_mut(), || {
        let flat: Vec<jint> = pdf_outline(h)?
            .iter()
            .flat_map(|i| [i.level as jint, i.page.map_or(-1, |p| p as jint)])
            .collect();
        unsafe {
            let new = (**env).NewIntArray.ok_or("JNI")?;
            let set = (**env).SetIntArrayRegion.ok_or("JNI")?;
            let arr = new(env, flat.len() as jint);
            if arr.is_null() {
                return Err("out of memory".into());
            }
            set(env, arr, 0, flat.len() as jint, flat.as_ptr());
            Ok(arr)
        }
    })
}

/// Frees what reading a PDF has accumulated (decoded page content, fonts
/// and images kept for redraws) by opening it afresh; call it from
/// `onTrimMemory`. Waits for a render in progress. Returns 0, or -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_trimMemory(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jint {
    guard(-1, || {
        with_handle(h, |hd| {
            match &hd.book {
                #[cfg(feature = "pdf")]
                Book::Pdf(pdf) => {
                    let fresh = pdf.trimmed().map_err(|e| e.to_string())?;
                    hd.book = Book::Pdf(fresh);
                }
                // A comic keeps one decoded page; drop it.
                #[cfg(feature = "cbz")]
                Book::Cbz(c) => {
                    c.trim();
                }
                #[allow(unreachable_patterns)]
                _ => {}
            }
            Ok(0)
        })
    })
}

/// Stops the tile batches running for `handle` before their next tile
/// (the user scrolled away). Never waits for them.
#[cfg(any(feature = "pdf", feature = "cbz"))]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_cancelRendering(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) {
    if let Some(g) = generation(h) {
        g.fetch_add(1, Ordering::Relaxed);
    }
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
            let flow = h.flow.get_or_insert_with(|| FlowState {
                fonts: font_set(),
                glyphs: GlyphCache::default(),
                pages: Vec::new(),
                width: 0,
                height: 0,
                chapter: 0,
                doc: Default::default(),
                #[cfg(feature = "pdf")]
                reflow: None,
            });
            let doc = match &h.book {
                #[cfg(feature = "pdf")]
                Book::Pdf(p) => reflow_chapter(p, &mut flow.reflow, chapter.max(0) as usize)?
                    .document
                    .clone(),
                #[allow(unreachable_patterns)]
                book => book
                    .flow()
                    .ok_or("not a reflowable book")?
                    .chapter(chapter.max(0) as usize)
                    .map_err(|e| e.to_string())?,
            };
            let opts = LayoutOptions {
                width: width as f32,
                height: height as f32,
                font_size,
                margin_x: width as f32 * 0.067,
                ..Default::default()
            };
            let images: Option<Box<dyn akkhor::flow::ImageProvider + '_>> = match &h.book {
                #[cfg(feature = "pdf")]
                Book::Pdf(p) => Some(Box::new(p.reflow_images())),
                #[allow(unreachable_patterns)]
                book => book.flow().and_then(|f| f.images(chapter.max(0) as usize)),
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
                #[cfg(feature = "pdf")]
                Book::Pdf(p) => Some(Box::new(p.reflow_images())),
                #[allow(unreachable_patterns)]
                book => book.flow().and_then(|f| f.images(flow.chapter)),
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
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => {
                let mut slot = hd.flow.as_mut().and_then(|f| f.reflow.take());
                let doc = reflow_chapter(p, &mut slot, chapter.max(0) as usize)
                    .map(|r| r.document.clone());
                if let Some(f) = hd.flow.as_mut() {
                    f.reflow = slot;
                }
                doc
            }
            #[allow(unreachable_patterns)]
            book => book
                .flow()
                .ok_or("not a reflowable book")?
                .chapter(chapter.max(0) as usize)
                .map_err(|e| e.to_string()),
        })?;
        let hits: Vec<jlong> = akkhor::flow::find_all(&doc, &query)
            .iter()
            .flat_map(|m| [pack(m.start), pack(m.end)])
            .collect();
        unsafe { long_array(env, &hits) }
    })
}

/// The reflow of a PDF chapter, kept in `slot` until another chapter is
/// asked for.
#[cfg(feature = "pdf")]
fn reflow_chapter<'s>(
    pdf: &akkhor::pdf::PdfDocument,
    slot: &'s mut Option<(usize, akkhor::pdf::reflow::Reflow)>,
    chapter: usize,
) -> Result<&'s akkhor::pdf::reflow::Reflow, String> {
    if slot.as_ref().is_none_or(|(c, _)| *c != chapter) {
        let range = pdf
            .reflow_chapters()
            .get(chapter)
            .cloned()
            .ok_or("reflow chapter out of range")?;
        *slot = Some((chapter, pdf.reflow(range).map_err(|e| e.to_string())?));
    }
    Ok(&slot.as_ref().expect("just set").1)
}

/// Runs `f` on the reflow of PDF chapter `chapter`.
#[cfg(feature = "pdf")]
fn with_reflow<T>(
    h: jlong,
    chapter: jint,
    f: impl FnOnce(&akkhor::pdf::reflow::Reflow) -> Result<T, String>,
) -> Result<T, String> {
    with_handle(h, |hd| {
        let pdf = match &hd.book {
            Book::Pdf(pdf) => pdf,
            #[allow(unreachable_patterns)]
            _ => return Err("not a PDF".into()),
        };
        let flow = hd.flow.get_or_insert_with(|| FlowState {
            fonts: font_set(),
            glyphs: GlyphCache::default(),
            pages: Vec::new(),
            width: 0,
            height: 0,
            chapter: 0,
            doc: Default::default(),
            reflow: None,
        });
        f(reflow_chapter(
            pdf,
            &mut flow.reflow,
            chapter.max(0) as usize,
        )?)
    })
}

/// How many reflow chapters a PDF has (its top-level outline entries, or
/// runs of 16 pages).
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_reflowChapterCount(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jint {
    guard(-1, || {
        with_handle(h, |hd| match &hd.book {
            Book::Pdf(p) => Ok(p.reflow_chapters().len() as jint),
            #[allow(unreachable_patterns)]
            _ => Err("not a PDF".into()),
        })
    })
}

/// The reflow chapter that holds PDF page `page`.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_reflowChapterOfPage(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    page: jint,
) -> jint {
    guard(-1, || {
        with_handle(h, |hd| match &hd.book {
            Book::Pdf(p) => p
                .reflow_chapters()
                .iter()
                .position(|r| r.contains(&(page.max(0) as usize)))
                .map(|c| c as jint)
                .ok_or_else(|| "page out of range".to_string()),
            #[allow(unreachable_patterns)]
            _ => Err("not a PDF".into()),
        })
    })
}

/// The reflow position of a PDF position ((page shl 32) or offset), as
/// (block shl 32) or offset in reflow chapter `chapter`; -1 if none.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_pdfToReflow(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    chapter: jint,
    pdf_pos: jlong,
) -> jlong {
    guard(-1, || {
        let (page, offset) = pdf_unpos(pdf_pos);
        with_reflow(h, chapter, |r| {
            Ok(r.from_page(page, offset)
                .map_or(-1, |(b, o)| pack(akkhor::flow::Position::new(b, o))))
        })
    })
}

/// The PDF position of a reflow position in chapter `chapter`; -1 if none.
#[cfg(feature = "pdf")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_reflowToPdf(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
    chapter: jint,
    pos: jlong,
) -> jlong {
    guard(-1, || {
        let p = unpack(pos);
        with_reflow(h, chapter, |r| {
            Ok(r.to_page(p.block, p.offset)
                .map_or(-1, |(page, o)| pdf_pos(page, o)))
        })
    })
}

/// UTF-16 offset in `text` of UTF-8 byte offset `utf8` (snapped back to a
/// character boundary).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_utf16Offset(
    env: *mut JNIEnv,
    _: jclass,
    text: jstring,
    utf8: jint,
) -> jint {
    guard(-1, || {
        let text = unsafe { jstr(env, text) }.ok_or("text is null")?;
        Ok(utf16_of(&text, utf8.max(0) as usize) as jint)
    })
}

/// UTF-8 byte offset in `text` of UTF-16 offset `utf16`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_utf8Offset(
    env: *mut JNIEnv,
    _: jclass,
    text: jstring,
    utf16: jint,
) -> jint {
    guard(-1, || {
        let text = unsafe { jstr(env, text) }.ok_or("text is null")?;
        Ok(utf8_of(&text, utf16.max(0) as usize) as jint)
    })
}

fn utf16_of(text: &str, utf8: usize) -> usize {
    let mut at = utf8.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    text[..at].encode_utf16().count()
}

pub(crate) fn utf8_of(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
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
        // An end before the start selects nothing.
        let rects = if end_page < page {
            Vec::new()
        } else {
            with_pdf_text(h, page, |t| {
                let b = if end_page > page {
                    t.text.len()
                } else {
                    b.min(t.text.len())
                };
                Ok(t.rects(a.min(b)..b))
            })?
        };
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

    #[cfg(feature = "pdf")]
    #[test]
    fn pdf_reflow_through_the_handle() {
        let path = format!(
            "{}/../../tests/corpus/books/pdf/bn.chrome.two.pdf",
            env!("CARGO_MANIFEST_DIR")
        );
        let font = format!(
            "{}/../../fonts/NotoSerifBengali-Regular.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        akkhor::pdf::register_reference_font(Source::open(font).unwrap().shared());
        let id = into_handle(akkhor::open(path).unwrap());
        let (n, null) = (std::ptr::null_mut(), std::ptr::null_mut());
        let chapters = unsafe { Java_dev_akkhor_Native_reflowChapterCount(n, null, id) };
        assert_eq!(chapters, 1);
        assert_eq!(
            unsafe { Java_dev_akkhor_Native_reflowChapterOfPage(n, null, id, 1) },
            0
        );
        let pages =
            unsafe { Java_dev_akkhor_Native_layoutChapter(n, null, id, 0, 720, 1280, 40.0) };
        assert!(pages >= 1, "{pages} pages");
        // Every block start maps to its page and back.
        let blocks = with_handle(id, |h| Ok(h.flow.as_ref().unwrap().doc.blocks.len())).unwrap();
        assert!(blocks > 10, "{blocks} blocks");
        for b in 0..blocks {
            let pos = pack(akkhor::flow::Position::new(b, 0));
            let pdf = unsafe { Java_dev_akkhor_Native_reflowToPdf(n, null, id, 0, pos) };
            assert!(pdf >= 0);
            let back = unsafe { Java_dev_akkhor_Native_pdfToReflow(n, null, id, 0, pdf) };
            let kind =
                with_handle(id, |h| Ok(h.flow.as_ref().unwrap().doc.blocks[b].kind)).unwrap();
            if kind != akkhor::flow::BlockKind::Image {
                assert_eq!(back, pos, "block {b}");
            }
        }
        assert_eq!(
            unsafe { Java_dev_akkhor_Native_reflowToPdf(n, null, id, 5, 0) },
            -1
        );
        unsafe { Java_dev_akkhor_Native_close(n, null, id) };
    }

    #[test]
    fn utf16_offsets() {
        let s = "অক্ষর 😀 ok";
        let at = s.find('😀').unwrap();
        assert_eq!(utf16_of(s, at), 6);
        assert_eq!(utf8_of(s, 6), at);
        assert_eq!(utf16_of(s, at + 1), 6); // inside a character: snapped back
        assert_eq!(utf8_of(s, 8), at + 4); // after the surrogate pair
        assert_eq!(utf8_of(s, 999), s.len());
    }

    #[cfg(feature = "pdf")]
    #[test]
    fn cancelling_never_waits_for_a_running_render() {
        let id = into_handle(akkhor::open(fixture("small.pdf")).unwrap());
        let g = generation(id).unwrap();
        let before = g.load(Ordering::Relaxed);
        // Hold the book's lock, as a render does: cancelling must not block.
        let shared = registry().get(&id).cloned().unwrap();
        let _busy = shared.lock().unwrap();
        unsafe {
            Java_dev_akkhor_Native_cancelRendering(std::ptr::null_mut(), std::ptr::null_mut(), id)
        };
        assert_eq!(g.load(Ordering::Relaxed), before + 1);
        drop(_busy);
        unsafe { Java_dev_akkhor_Native_close(std::ptr::null_mut(), std::ptr::null_mut(), id) };
        assert!(generation(id).is_none());
        // Cancelling a closed handle is harmless.
        unsafe {
            Java_dev_akkhor_Native_cancelRendering(std::ptr::null_mut(), std::ptr::null_mut(), id)
        };
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
