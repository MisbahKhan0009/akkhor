//! Times building recovery tables (debug helper).
use std::sync::Arc;
use std::time::Instant;
fn main() {
    for path in std::env::args().skip(1) {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(std::fs::read(&path).unwrap());
        let t = Instant::now();
        let deep = std::env::var("DEEP").is_ok();
        let r = akkhor_pdf::recover::build_with(&data, true, deep).unwrap();
        if let Ok(ids) = std::env::var("IDS") {
            for id in ids.split(',') {
                let g: u32 = id.parse().unwrap();
                println!("  {g}: {:?}", r.map.get(&g));
            }
        }
        if std::env::var("SEQ").is_ok() {
            let mut v: Vec<_> = r.sequences.iter().collect();
            v.sort();
            for (k, t) in v.iter().step_by(v.len() / 40 + 1) {
                let parts: Vec<String> = k
                    .iter()
                    .map(|g| r.map.get(g).map(|x| x.text.clone()).unwrap_or("?".into()))
                    .collect();
                println!("  {k:?} {parts:?} => {t:?}");
            }
        }
        println!(
            "{path}: {:.1} ms, {} glyphs, {} sequences",
            t.elapsed().as_secs_f64() * 1e3,
            r.map.len(),
            r.sequences.len()
        );
    }
}
// SEQ=1 prints sequences with the glyphs' single texts.
