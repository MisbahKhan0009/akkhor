//! The zip reader: list the entries and read each.
#![no_main]
use akkhor_core::Source;
use akkhor_epub::zip::ZipArchive;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(zip) = ZipArchive::open(Source::from_vec(data.to_vec())) else {
        return;
    };
    let names: Vec<String> = zip.entries().iter().map(|e| e.name.clone()).collect();
    for name in names.iter().take(16) {
        let _ = zip.read(name);
    }
});
