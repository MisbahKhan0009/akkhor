//! A text file: encoding, Bijoy detection and conversion, layout.
#![no_main]
use akkhor_flow::{Document, bijoy, decode_text, encoding};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let decoded = decode_text(data);
    if encoding::looks_like_bijoy(&decoded.text) {
        std::hint::black_box(bijoy::bijoy_to_unicode(&decoded.text));
    }
    let doc = Document::from_plain_text(&decoded.text);
    akkhor_fuzz::lay_out(&doc);
});
