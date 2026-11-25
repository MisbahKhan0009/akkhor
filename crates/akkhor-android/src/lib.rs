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
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Mutex;

use akkhor::{Book, Format, Source, Theme};
use jni_sys::{JNIEnv, jclass, jfloat, jint, jlong, jobject, jstring};

#[cfg(feature = "flow")]
use akkhor::flow::{FontSet, GlyphCache, LayoutOptions, Page, Script, fonts::FontData};

static FONTS: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_error(msg: impl ToString) {
    let c = CString::new(msg.to_string().replace('\0', " ")).ok();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
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

unsafe fn jstr(env: *mut JNIEnv, s: jstring) -> Option<String> {
    if s.is_null() {
        return None;
    }
    unsafe {
        let get = (**env).GetStringUTFChars?;
        let release = (**env).ReleaseStringUTFChars?;
        let chars = get(env, s, std::ptr::null_mut());
        if chars.is_null() {
            return None;
        }
        let out = CStr::from_ptr(chars).to_string_lossy().into_owned();
        release(env, s, chars);
        Some(out)
    }
}

unsafe fn handle<'a>(h: jlong) -> Result<&'a mut Handle, String> {
    if h == 0 {
        return Err("closed or invalid handle".into());
    }
    Ok(unsafe { &mut *(h as *mut Handle) })
}

fn into_handle(book: Book) -> jlong {
    Box::into_raw(Box::new(Handle {
        book,
        #[cfg(feature = "flow")]
        flow: None,
    })) as jlong
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
    if h != 0 {
        let _ = catch_unwind(|| drop(unsafe { Box::from_raw(h as *mut Handle) }));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_format(
    _: *mut JNIEnv,
    _: jclass,
    h: jlong,
) -> jint {
    guard(-1, || {
        let h = unsafe { handle(h) }?;
        Ok(match h.book.format() {
            Format::Pdf => 0,
            Format::Epub => 1,
            Format::Txt => 2,
            _ => 9,
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
        let h = unsafe { handle(h) }?;
        Ok(match &h.book {
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => p.page_count() as jint,
            #[cfg(feature = "epub")]
            Book::Epub(e) => e.chapter_count() as jint,
            #[cfg(feature = "txt")]
            Book::Text(_) => 1,
        })
    })
}

#[cfg(feature = "pdf")]
fn pdf_size(h: jlong, page: jint) -> Result<(f32, f32), String> {
    let h = unsafe { handle(h) }?;
    match &h.book {
        Book::Pdf(p) => p.page_size(page as usize).map_err(|e| e.to_string()),
        #[allow(unreachable_patterns)]
        _ => Err("not a PDF".into()),
    }
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
        let h = unsafe { handle(h) }?;
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
        let _ = chapter; // unused in TXT-only builds
        let h = unsafe { handle(h) }?;
        let doc = match &h.book {
            #[cfg(feature = "epub")]
            Book::Epub(e) => e.chapter(chapter as usize).map_err(|e| e.to_string())?,
            #[cfg(feature = "txt")]
            Book::Text(d) => d.clone(),
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
        flow.width = width.max(0) as u32;
        flow.height = height.max(0) as u32;
        Ok(flow.pages.len() as jint)
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
        let h = unsafe { handle(h) }?;
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
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_akkhor_Native_lastError(
    env: *mut JNIEnv,
    _: jclass,
) -> jstring {
    LAST_ERROR.with(|e| match e.borrow().as_ref() {
        Some(msg) => unsafe {
            match (**env).NewStringUTF {
                Some(new) => new(env, msg.as_ptr() as *const c_char),
                None => std::ptr::null_mut(),
            }
        },
        None => std::ptr::null_mut(),
    })
}
