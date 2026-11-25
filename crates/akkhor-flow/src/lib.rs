//! Reflowable text for Akkhor.
//!
//! Every reflowable format (EPUB, MOBI/AZW3, FB2, TXT) is converted into one
//! small [`Document`] model. Everything after that is shared:
//!
//! 1. [`script`]: which script each character belongs to, and where the
//!    orthographic syllables (aksharas) are. A conjunct such as ক্ষ or द्ध is
//!    one unit: lines, selections and cursors never split it.
//! 2. [`fonts`]: one font per script, loaded only the first time a book
//!    actually uses that script.
//! 3. [`shape`]: OpenType shaping (HarfRust), so conjuncts, reph, pre-base
//!    vowel signs and other Indic forms come out right.
//! 4. [`layout`]: line breaking and pagination.
//! 5. [`paint`]: drawing a page into a pixel buffer.

pub mod fonts;
pub mod layout;
pub mod markup;
pub mod model;
pub mod paint;
pub mod script;
pub mod search;
pub mod select;
pub mod shape;

pub use fonts::{FontId, FontSet};
pub use layout::{LayoutOptions, Line, Page, PlacedGlyph, paginate};
pub use model::{Block, BlockKind, Document, Position, Style, StyleSpan};
pub use paint::{GlyphCache, render_page};
pub use script::Script;
pub use search::{Match, find_all, snippet};
pub use select::{Rect, hit_test, selection_rects, text_between, word_at};
