//! What a library shows about a book before it is read: title, authors,
//! language, length and cover, read without opening chapters or pages
//! beyond the first.

use crate::{Book, Format, json};

/// A book's description, from its own metadata where it has some, guessed
/// from its first text where it does not.
#[derive(Clone, Debug, PartialEq)]
pub struct BookInfo {
    pub format: Format,
    /// None when the book names none (the app shows the file name).
    pub title: Option<String>,
    pub authors: Vec<String>,
    /// A language tag (`bn`, `hi`, `en`, ...).
    pub language: Option<String>,
    /// The language was guessed from the script of the first text.
    pub language_guessed: bool,
    pub publisher: Option<String>,
    /// PDF pages, EPUB chapters, 1 for a text file.
    pub length: usize,
    pub has_cover: bool,
    /// The book can be shown as reflowed text (EPUB and text always; a PDF
    /// when its first pages have text, not only pictures of pages).
    pub reflowable: bool,
    /// PDFs with Bangla or Devanagari text: how sure their text is, 0 to 1
    /// (see `pdf::PdfDocument::text_quality`).
    pub text_quality: Option<f32>,
    /// The text needs a font the engine does not have: offer a font pack.
    pub needs_fonts: bool,
    /// A text file typed in Bijoy, shown converted to Unicode.
    pub bijoy: bool,
    /// The series the book belongs to (FB2, comics), with its number.
    pub series: Option<String>,
    /// Pages are read right to left (manga).
    pub right_to_left: bool,
}

/// A cover image, RGBA, unpremultiplied.
#[derive(Clone, Debug, PartialEq)]
pub struct Cover {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl BookInfo {
    pub fn to_json(&self) -> String {
        json::Obj::new()
            .str(
                "format",
                match self.format {
                    Format::Pdf => "pdf",
                    Format::Epub => "epub",
                    Format::Txt => "txt",
                    Format::Mobi => "mobi",
                    Format::Fb2 => "fb2",
                    Format::Cbz => "cbz",
                },
            )
            .opt_str("title", self.title.as_deref())
            .strs("authors", &self.authors)
            .opt_str("language", self.language.as_deref())
            .bool("language_guessed", self.language_guessed)
            .opt_str("publisher", self.publisher.as_deref())
            .num("length", self.length as f64)
            .bool("has_cover", self.has_cover)
            .bool("reflowable", self.reflowable)
            .opt_num("text_quality", self.text_quality.map(f64::from))
            .bool("needs_fonts", self.needs_fonts)
            .bool("bijoy", self.bijoy)
            .opt_str("series", self.series.as_deref())
            .str("direction", if self.right_to_left { "rtl" } else { "ltr" })
            .end()
    }
}

/// Pages a PDF's description is read from.
#[cfg(feature = "pdf")]
const FIRST_PAGES: usize = 3;

/// The language most of `text` is written in, by script: `bn` for
/// Bengali, `hi` for Devanagari, `en` for Latin, and so on; `None` when
/// there are too few letters to tell.
pub fn guess_language(text: &str) -> Option<String> {
    use crate::flow::Script;
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for c in text.chars().filter(|c| c.is_alphabetic()).take(20_000) {
        let s = crate::flow::script::script_of(c);
        match counts.iter_mut().find(|(x, _)| *x == s) {
            Some((_, n)) => *n += 1,
            None => counts.push((s, 1)),
        }
    }
    let (script, n) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    if n < 20 {
        return None;
    }
    Some(
        match script {
            Script::Bengali => "bn",
            Script::Devanagari => "hi",
            Script::Gurmukhi => "pa",
            Script::Gujarati => "gu",
            Script::Oriya => "or",
            Script::Tamil => "ta",
            Script::Telugu => "te",
            Script::Kannada => "kn",
            Script::Malayalam => "ml",
            Script::Latin => "en",
            _ => return None,
        }
        .to_string(),
    )
}

impl Book {
    /// The book's description for a library.
    pub fn info(&self) -> BookInfo {
        match self {
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => pdf_info(p),
            #[cfg(feature = "epub")]
            Book::Epub(e) => {
                let m = &e.metadata;
                let (language, language_guessed) = match &m.language {
                    Some(l) => (Some(l.clone()), false),
                    None => {
                        let text = e
                            .chapter(0)
                            .map(|d| d.blocks.iter().map(|b| b.text.as_str()).collect::<String>())
                            .unwrap_or_default();
                        (guess_language(&text), true)
                    }
                };
                BookInfo {
                    format: Format::Epub,
                    title: m.title.clone(),
                    authors: m.creators.clone(),
                    language,
                    language_guessed,
                    publisher: m.publisher.clone(),
                    length: e.chapter_count(),
                    has_cover: m.cover.is_some(),
                    reflowable: true,
                    text_quality: None,
                    needs_fonts: false,
                    bijoy: false,
                    series: None,
                    right_to_left: false,
                }
            }
            #[cfg(feature = "mobi")]
            Book::Mobi(b) => {
                let m = &b.metadata;
                flow_info(
                    self,
                    Format::Mobi,
                    m.title.clone(),
                    m.authors.clone(),
                    m.language.clone(),
                    m.publisher.clone(),
                    b.cover_bytes().is_some(),
                    None,
                )
            }
            #[cfg(feature = "fb2")]
            Book::Fb2(b) => {
                let m = &b.metadata;
                flow_info(
                    self,
                    Format::Fb2,
                    m.title.clone(),
                    m.authors.clone(),
                    m.language.clone(),
                    m.publisher.clone(),
                    m.cover.is_some(),
                    m.series.clone(),
                )
            }
            #[cfg(feature = "cbz")]
            Book::Cbz(c) => {
                let m = &c.metadata;
                BookInfo {
                    format: Format::Cbz,
                    title: m.title.clone(),
                    authors: m.writers.clone(),
                    language: m.language.clone(),
                    language_guessed: false,
                    publisher: None,
                    length: c.page_count(),
                    has_cover: true,
                    reflowable: false,
                    text_quality: None,
                    needs_fonts: false,
                    bijoy: false,
                    series: m.series.as_ref().map(|s| match &m.number {
                        Some(n) => format!("{s} {n}"),
                        None => s.clone(),
                    }),
                    right_to_left: m.right_to_left,
                }
            }
            #[cfg(feature = "txt")]
            Book::Text(t) => {
                let first: String = t
                    .document
                    .blocks
                    .iter()
                    .map(|b| b.text.trim())
                    .find(|s| !s.is_empty())
                    .map(|s| s.chars().take(80).collect())
                    .unwrap_or_default();
                let sample: String = t
                    .document
                    .blocks
                    .iter()
                    .take(200)
                    .map(|b| b.text.as_str())
                    .collect();
                BookInfo {
                    format: Format::Txt,
                    title: (!first.is_empty()).then_some(first),
                    authors: Vec::new(),
                    language: guess_language(&sample),
                    language_guessed: true,
                    publisher: None,
                    length: 1,
                    has_cover: false,
                    reflowable: true,
                    text_quality: None,
                    needs_fonts: false,
                    bijoy: t.encoding == crate::flow::TextEncoding::LegacyBijoy,
                    series: None,
                    right_to_left: false,
                }
            }
        }
    }

    /// The cover, fitted inside `max_width` × `max_height`: an EPUB's cover
    /// image, a PDF's first page; `None` for text files and EPUBs with no
    /// cover (the app draws its own).
    pub fn cover(&self, max_width: u32, max_height: u32) -> Option<Cover> {
        let (mw, mh) = (max_width.max(1), max_height.max(1));
        let _ = (mw, mh); // unused in text-only builds
        match self {
            #[cfg(feature = "pdf")]
            Book::Pdf(p) => {
                let (w, h) = p.page_size(0).ok()?;
                let scale = (mw as f32 / w.max(1.0)).min(mh as f32 / h.max(1.0));
                let page = p.render_page(0, scale, &crate::Theme::ORIGINAL).ok()?;
                Some(Cover {
                    width: page.width,
                    height: page.height,
                    rgba: page.into_unpremultiplied(),
                })
            }
            #[cfg(feature = "cbz")]
            Book::Cbz(c) => fit(&c.cover_bytes()?, mw, mh),
            #[cfg(feature = "mobi")]
            Book::Mobi(b) => fit(&b.cover_bytes()?, mw, mh),
            #[cfg(feature = "fb2")]
            Book::Fb2(b) => fit(&b.cover_bytes()?, mw, mh),
            #[cfg(feature = "epub")]
            Book::Epub(e) => {
                let bytes = e.cover_bytes()?;
                let img = crate::flow::image::decode(&bytes)?;
                let scale = (mw as f32 / img.width.max(1) as f32)
                    .min(mh as f32 / img.height.max(1) as f32)
                    .min(1.0);
                let (w, h) = (
                    ((img.width as f32 * scale).round() as u32).max(1),
                    ((img.height as f32 * scale).round() as u32).max(1),
                );
                Some(Cover {
                    width: w,
                    height: h,
                    rgba: crate::flow::image::resize(&img, w, h),
                })
            }
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }
}

/// A picture fitted inside `mw` x `mh`, never enlarged.
#[allow(dead_code)]
fn fit(bytes: &[u8], mw: u32, mh: u32) -> Option<Cover> {
    // Comic covers may be big scans.
    let img = crate::flow::image::decode_up_to(bytes, 40_000_000)?;
    let scale = (mw as f32 / img.width.max(1) as f32)
        .min(mh as f32 / img.height.max(1) as f32)
        .min(1.0);
    let (w, h) = (
        ((img.width as f32 * scale).round() as u32).max(1),
        ((img.height as f32 * scale).round() as u32).max(1),
    );
    Some(Cover {
        width: w,
        height: h,
        rgba: crate::flow::image::resize(&img, w, h),
    })
}

/// A reflowable book's description from its metadata; the language is
/// guessed from the first chapter when the book names none.
#[allow(dead_code, clippy::too_many_arguments)]
fn flow_info(
    book: &Book,
    format: Format,
    title: Option<String>,
    authors: Vec<String>,
    language: Option<String>,
    publisher: Option<String>,
    has_cover: bool,
    series: Option<String>,
) -> BookInfo {
    let flow = book.flow();
    let (language, language_guessed) = match language {
        Some(l) => (Some(l), false),
        None => {
            let text = flow
                .and_then(|f| f.chapter(0).ok())
                .map(|d| d.blocks.iter().map(|b| b.text.as_str()).collect::<String>())
                .unwrap_or_default();
            (guess_language(&text), true)
        }
    };
    BookInfo {
        format,
        title,
        authors,
        language,
        language_guessed,
        publisher,
        length: flow.map_or(0, |f| f.chapter_count()),
        has_cover,
        reflowable: true,
        text_quality: None,
        needs_fonts: false,
        bijoy: false,
        series,
        right_to_left: false,
    }
}

#[cfg(feature = "pdf")]
fn pdf_info(p: &crate::pdf::PdfDocument) -> BookInfo {
    let own = p.info();
    let first = 0..FIRST_PAGES.min(p.page_count());
    let texts: Vec<_> = first
        .clone()
        .filter_map(|i| p.page_text_cached(i).ok())
        .collect();
    let all: String = texts.iter().map(|t| t.text.as_str()).collect();
    // A title the PDF does not give: the largest line on the first page.
    let title = own.title.clone().or_else(|| {
        let t = texts.first()?;
        let mut sizes: Vec<f32> = t.line_info.iter().map(|l| l.size).collect();
        sizes.sort_by(f32::total_cmp);
        let median = *sizes.get(sizes.len() / 2)?;
        t.lines
            .iter()
            .zip(&t.line_info)
            .filter(|(r, l)| {
                let s = t.text[(*r).clone()].trim();
                l.size >= median * 1.3 && (3..=120).contains(&s.chars().count())
            })
            .max_by(|a, b| a.1.size.total_cmp(&b.1.size))
            .map(|(r, _)| t.text[r.clone()].trim().to_string())
    });
    let quality = p.text_quality(first);
    BookInfo {
        format: Format::Pdf,
        title,
        authors: own.author.into_iter().collect(),
        language: guess_language(&all),
        language_guessed: true,
        publisher: None,
        length: p.page_count(),
        has_cover: p.page_count() > 0,
        reflowable: all.chars().filter(|c| c.is_alphabetic()).count() >= 20,
        text_quality: quality,
        needs_fonts: quality.is_some_and(|q| q < crate::pdf::TEXT_QUALITY_GOOD),
        bijoy: false,
        series: None,
        right_to_left: false,
    }
}
