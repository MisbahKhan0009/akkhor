//! A fingerprint of the code that builds reference-font tables, so tables
//! kept on disk by an older engine are never used by a newer one.

use std::hash::Hasher;

fn main() {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    for f in [
        "src/recover.rs",
        "src/shape.rs",
        "src/reference.rs",
        "../../Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={f}");
        if let Ok(b) = std::fs::read(f) {
            h.write(&b);
        }
    }
    h.write(env!("CARGO_PKG_VERSION").as_bytes());
    println!("cargo:rustc-env=AKKHOR_TABLES={:016x}", h.0);
}

struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}
