//! Android: pack the dynamic relocations (Android 6+, our minimum is 7) and
//! fold identical functions. Both shrink libakkhor.so without touching code.
//! Set here rather than in .cargo/config.toml, which a RUSTFLAGS variable
//! (as CI uses) would silently override.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,--pack-dyn-relocs=android");
        println!("cargo:rustc-cdylib-link-arg=-Wl,--icf=all");
    }
}
