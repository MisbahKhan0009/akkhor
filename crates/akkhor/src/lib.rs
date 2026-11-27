//! # Akkhor
//!
//! A tiny, fast reading engine that gets Indic scripts right.
//!
//! ```no_run
//! let book = akkhor::open("book.epub")?;
//! println!("{:?}", book.format());
//! # Ok::<(), akkhor::Error>(())
//! ```
//!
//! Formats are Cargo features (`pdf`, `epub`, `txt`, `mobi`, `fb2`, `cbz`).
//! Opening a format that was switched off returns [`Error::FormatDisabled`].

pub use akkhor_core::{Error, Format, LruCache, Result, Rgba, Source, Theme, Tile, sniff};

#[cfg(feature = "flow")]
pub use akkhor_flow as flow;

#[cfg(feature = "pdf")]
pub use akkhor_pdf as pdf;

#[cfg(feature = "epub")]
pub use akkhor_epub as epub;

#[cfg(feature = "mobi")]
pub use akkhor_mobi as mobi;

#[cfg(feature = "fb2")]
pub use akkhor_fb2 as fb2;

#[cfg(feature = "cbz")]
pub use akkhor_cbz as cbz;

use std::path::Path;

#[cfg(feature = "flow")]
mod flowbook;
#[cfg(feature = "flow")]
pub mod info;
pub mod json;
#[cfg(feature = "flow")]
pub mod reading;
#[cfg(feature = "flow")]
pub mod search;

#[cfg(feature = "flow")]
pub use flowbook::FlowBook;
#[cfg(feature = "flow")]
pub use info::{BookInfo, Cover};
#[cfg(feature = "flow")]
pub use reading::{Locator, Reading};

/// An open book of any enabled format.
pub enum Book {
    #[cfg(feature = "pdf")]
    Pdf(akkhor_pdf::PdfDocument),
    #[cfg(feature = "epub")]
    Epub(akkhor_epub::EpubBook),
    #[cfg(feature = "txt")]
    Text(TextBook),
    #[cfg(feature = "mobi")]
    Mobi(akkhor_mobi::MobiBook),
    #[cfg(feature = "fb2")]
    Fb2(akkhor_fb2::Fb2Book),
    #[cfg(feature = "cbz")]
    Cbz(akkhor_cbz::CbzBook),
}

/// A plain-text book, with the encoding it was read in.
#[cfg(feature = "txt")]
pub struct TextBook {
    pub document: akkhor_flow::Document,
    /// `LegacyBijoy` means the file was Bangla typed with Bijoy; the document
    /// holds it converted to Unicode.
    pub encoding: akkhor_flow::TextEncoding,
}

impl Book {
    pub fn format(&self) -> Format {
        match *self {
            #[cfg(feature = "pdf")]
            Book::Pdf(_) => Format::Pdf,
            #[cfg(feature = "epub")]
            Book::Epub(_) => Format::Epub,
            #[cfg(feature = "txt")]
            Book::Text(_) => Format::Txt,
            #[cfg(feature = "mobi")]
            Book::Mobi(_) => Format::Mobi,
            #[cfg(feature = "fb2")]
            Book::Fb2(_) => Format::Fb2,
            #[cfg(feature = "cbz")]
            Book::Cbz(_) => Format::Cbz,
        }
    }

    /// Whether the book is pictures of pages (PDF, CBZ) rather than
    /// reflowable text.
    pub fn is_paged(&self) -> bool {
        matches!(self.format(), Format::Pdf | Format::Cbz)
    }
}

/// Containers that are recognised but not read, by their first bytes.
fn unsupported(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\xeaDRMION\xee") {
        return Some("KFX (DRM)");
    }
    if b.starts_with(b"CONT\x02\x00") || b.starts_with(b"CONT\x01\x00") {
        return Some("KFX");
    }
    if b.starts_with(b"TPZ") {
        return Some("Topaz");
    }
    if b.starts_with(b"Rar!") {
        return Some("CBR (RAR)");
    }
    if b.starts_with(b"7z\xbc\xaf") {
        return Some("CB7 (7-Zip)");
    }
    None
}

/// Opens a book from a file path. The file is memory-mapped, not read.
pub fn open(path: impl AsRef<Path>) -> Result<Book> {
    let path = path.as_ref();
    let source = Source::open(path)?;
    open_source(source, path.file_name().and_then(|n| n.to_str()))
}

/// Opens a book from a [`Source`]. `name` helps tell zip-based formats apart.
pub fn open_source(source: Source, name: Option<&str>) -> Result<Book> {
    if let Some(what) = unsupported(source.bytes()) {
        return Err(Error::Unsupported(what));
    }
    let format = sniff(source.bytes(), name).ok_or(Error::UnknownFormat)?;
    match format {
        #[cfg(feature = "pdf")]
        Format::Pdf => Ok(Book::Pdf(akkhor_pdf::PdfDocument::open(&source)?)),
        #[cfg(feature = "epub")]
        Format::Epub => Ok(Book::Epub(akkhor_epub::EpubBook::open(source)?)),
        #[cfg(feature = "txt")]
        Format::Txt => {
            let decoded = akkhor_flow::decode_text(source.bytes());
            Ok(Book::Text(TextBook {
                document: akkhor_flow::Document::from_plain_text(&decoded.text),
                encoding: decoded.encoding,
            }))
        }
        #[cfg(feature = "mobi")]
        Format::Mobi => Ok(Book::Mobi(akkhor_mobi::MobiBook::open(source)?)),
        #[cfg(feature = "fb2")]
        Format::Fb2 => Ok(Book::Fb2(akkhor_fb2::Fb2Book::open(source)?)),
        #[cfg(feature = "cbz")]
        Format::Cbz => Ok(Book::Cbz(akkhor_cbz::CbzBook::open(source)?)),
        #[allow(unreachable_patterns)]
        other => Err(Error::FormatDisabled(other)),
    }
}
