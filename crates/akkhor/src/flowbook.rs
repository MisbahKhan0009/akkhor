//! One interface over the reflowable formats (EPUB, MOBI, FB2, text and
//! PDF reflow chapters aside): chapters as documents, their pictures, and
//! how big each chapter is.

use akkhor_core::Result;
use akkhor_flow::Document;
use akkhor_flow::image::ImageProvider;

use crate::Book;

/// A book of reflowable chapters.
pub trait FlowBook {
    fn chapter_count(&self) -> usize;
    /// Chapter `index` as a document.
    fn chapter(&self, index: usize) -> Result<Document>;
    /// The pictures of chapter `index`, if the format has any.
    fn images(&self, index: usize) -> Option<Box<dyn ImageProvider + '_>>;
    /// A size for each chapter (bytes of text, roughly), for progress.
    fn chapter_sizes(&self) -> Vec<u64>;
}

#[cfg(feature = "epub")]
impl FlowBook for akkhor_epub::EpubBook {
    fn chapter_count(&self) -> usize {
        self.chapter_count()
    }
    fn chapter(&self, index: usize) -> Result<Document> {
        self.chapter(index)
    }
    fn images(&self, index: usize) -> Option<Box<dyn ImageProvider + '_>> {
        Some(Box::new(self.chapter_images(index)))
    }
    fn chapter_sizes(&self) -> Vec<u64> {
        self.chapter_sizes()
    }
}

#[cfg(feature = "mobi")]
impl FlowBook for akkhor_mobi::MobiBook {
    fn chapter_count(&self) -> usize {
        self.chapter_count()
    }
    fn chapter(&self, index: usize) -> Result<Document> {
        self.chapter(index)
    }
    fn images(&self, index: usize) -> Option<Box<dyn ImageProvider + '_>> {
        Some(Box::new(self.chapter_images(index)))
    }
    fn chapter_sizes(&self) -> Vec<u64> {
        self.chapter_sizes()
    }
}

#[cfg(feature = "fb2")]
impl FlowBook for akkhor_fb2::Fb2Book {
    fn chapter_count(&self) -> usize {
        self.chapter_count()
    }
    fn chapter(&self, index: usize) -> Result<Document> {
        self.chapter(index)
    }
    fn images(&self, index: usize) -> Option<Box<dyn ImageProvider + '_>> {
        Some(Box::new(self.chapter_images(index)))
    }
    fn chapter_sizes(&self) -> Vec<u64> {
        self.chapter_sizes()
    }
}

#[cfg(feature = "txt")]
impl FlowBook for crate::TextBook {
    fn chapter_count(&self) -> usize {
        1
    }
    fn chapter(&self, index: usize) -> Result<Document> {
        if index == 0 {
            Ok(self.document.clone())
        } else {
            Err(akkhor_core::Error::OutOfRange)
        }
    }
    fn images(&self, _index: usize) -> Option<Box<dyn ImageProvider + '_>> {
        None
    }
    fn chapter_sizes(&self) -> Vec<u64> {
        vec![1]
    }
}

impl Book {
    /// The book as reflowable chapters: EPUB, MOBI, FB2 and text files.
    /// PDFs reflow through [`crate::Reading`]; comics do not reflow.
    pub fn flow(&self) -> Option<&dyn FlowBook> {
        match self {
            #[cfg(feature = "epub")]
            Book::Epub(b) => Some(b),
            #[cfg(feature = "mobi")]
            Book::Mobi(b) => Some(b),
            #[cfg(feature = "fb2")]
            Book::Fb2(b) => Some(b),
            #[cfg(feature = "txt")]
            Book::Text(b) => Some(b),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }
}
