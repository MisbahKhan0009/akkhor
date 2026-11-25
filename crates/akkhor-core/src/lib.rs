//! Shared building blocks of the Akkhor reading engine.
//!
//! Nothing in here knows about a specific format. Format crates build on
//! these pieces:
//!
//! - [`Source`]: a book's bytes, memory-mapped so the file is never read
//!   into RAM as a whole. The OS pages in only what a parser touches.
//! - [`Format`] and [`sniff`]: cheap detection from the first bytes.
//! - [`LruCache`]: a cache with a byte budget, so every cache in the engine
//!   has a hard ceiling.
//! - [`Theme`]: reading colors (paper, sepia, dusk, night).

mod cache;
mod error;
mod format;
mod source;
mod theme;

pub use cache::LruCache;
pub use error::{Error, Result};
pub use format::{Format, sniff};
pub use source::{Bytes, Source};
pub use theme::{Rgba, Theme};

/// A rectangle in device pixels, used to address one tile of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Tile {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Bytes an RGBA8 buffer for this tile needs.
    pub fn rgba_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}
