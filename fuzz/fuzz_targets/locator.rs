//! Stored places: anything that parses prints back to the same place.
#![no_main]
use akkhor::Locator;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(l) = Locator::parse(s) {
        let again = Locator::parse(&l.to_string()).expect("a printed locator parses");
        assert_eq!(again, l);
    }
});
