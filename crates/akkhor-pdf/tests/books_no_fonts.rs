//! Typst writes its own text layer (ToUnicode maps glyphs to whole clusters,
//! ActualText spans cut mid-cluster): with no font known to Akkhor at all,
//! its words must still come out whole. A separate test binary, so no
//! reference font is registered.

mod common;

#[test]
fn typst_words_whole_without_fonts() {
    common::check_books(|n| n.contains("typst"), |_| 1.0);
}
