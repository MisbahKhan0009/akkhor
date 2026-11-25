//! Words on the book-layout corpus (tests/corpus/books): every word of the
//! ground truth must come out of the page text whole, not run into its
//! neighbour or split by letters the producer repeated. Line-end hyphens are
//! joined first: they are a reflow question (Phase 4.3), not a text-layer
//! one.

mod common;

use common::*;

#[test]
fn book_words_come_out_whole() {
    register_test_fonts();
    check_books(
        |_| true,
        |name| if name.contains("mukta") { 0.99 } else { 1.0 },
    );
}
