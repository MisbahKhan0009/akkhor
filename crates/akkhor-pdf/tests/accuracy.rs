//! Text-layer accuracy on the corpus in tests/corpus: character error rate
//! (edit distance / length, ignoring whitespace) against the ground truth.

mod common;

use common::*;

#[test]
fn corpus_character_error_rates() {
    register_test_fonts();
    assert_targets(&run_corpus());
}

#[test]
fn edit_distance_works() {
    let c = |s: &str| s.chars().collect::<Vec<_>>();
    assert_eq!(edit_distance(&c("kitten"), &c("sitting")), 3);
    assert_eq!(cer("ক্ষমা", "ক্ষমা"), 0.0);
}
