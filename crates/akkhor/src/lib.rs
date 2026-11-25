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
//! Formats are Cargo features (`pdf`, `epub`, `txt`). Opening a format that
//! was switched off returns [`Error::FormatDisabled`].

pub use akkhor_core::{Error, Format, LruCache, Result, Rgba, Source, Theme, Tile, sniff};

#[cfg(feature = "flow")]
pub use akkhor_flow as flow;

#[cfg(feature = "pdf")]
pub use akkhor_pdf as pdf;

#[cfg(feature = "epub")]
pub use akkhor_epub as epub;

use std::path::Path;

/// An open book of any enabled format.
pub enum Book {
    #[cfg(feature = "pdf")]
    Pdf(akkhor_pdf::PdfDocument),
    #[cfg(feature = "epub")]
    Epub(akkhor_epub::EpubBook),
    #[cfg(feature = "txt")]
    Text(akkhor_flow::Document),
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
        }
    }
}

/// Opens a book from a file path. The file is memory-mapped, not read.
pub fn open(path: impl AsRef<Path>) -> Result<Book> {
    let path = path.as_ref();
    let source = Source::open(path)?;
    open_source(source, path.file_name().and_then(|n| n.to_str()))
}

/// Opens a book from a [`Source`]. `name` helps tell zip-based formats apart.
pub fn open_source(source: Source, name: Option<&str>) -> Result<Book> {
    let format = sniff(source.bytes(), name).ok_or(Error::UnknownFormat)?;
    match format {
        #[cfg(feature = "pdf")]
        Format::Pdf => Ok(Book::Pdf(akkhor_pdf::PdfDocument::open(&source)?)),
        #[cfg(feature = "epub")]
        Format::Epub => Ok(Book::Epub(akkhor_epub::EpubBook::open(source)?)),
        #[cfg(feature = "txt")]
        Format::Txt => {
            let text = String::from_utf8_lossy(source.bytes());
            Ok(Book::Text(akkhor_flow::Document::from_plain_text(&text)))
        }
        other => Err(Error::FormatDisabled(other)),
    }
}
