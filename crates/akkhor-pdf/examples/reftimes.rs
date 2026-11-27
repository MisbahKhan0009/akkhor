//! How long each reference-font table takes to build (or to read back
//! from a cache folder), per bundled font.
//!   cargo run --release -p akkhor-pdf --features bundled-fonts --example reftimes [CACHE_DIR]
fn main() {
    if let Some(dir) = std::env::args().nth(1) {
        akkhor_pdf::set_cache_dir(dir);
    }
    akkhor_pdf::reference::print_build_times();
}
