//! The accuracy corpus again with glyph names ignored: phone fonts often
//! ship without them, so recovery must stand on shaping alone. (A separate
//! test binary, because reference tables are built once per process.)

mod common;

use common::*;

#[test]
fn corpus_without_glyph_names() {
    akkhor_pdf::reference::set_glyph_names(false);
    register_test_fonts();
    assert_targets(&run_corpus());
}
