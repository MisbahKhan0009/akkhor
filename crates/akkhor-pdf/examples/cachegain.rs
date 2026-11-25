//! Measures re-render time with a fresh vs a shared hayro RenderCache.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::RenderContext;
use hayro::{RenderCache, RenderSettings, render_into};
use std::sync::Arc;
use std::time::Instant;
fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().unwrap();
    let page: usize = a.next().unwrap().parse().unwrap();
    let scale: f64 = a.next().unwrap().parse().unwrap();
    let data = std::fs::read(&path).unwrap();
    let pdf: &'static Pdf = Box::leak(Box::new(Pdf::new(Arc::new(data)).unwrap()));
    let p = pdf.pages().get(page).unwrap();
    let (w, h) = p.render_dimensions();
    let (w, h) = ((w as f64 * scale) as u16, (h as f64 * scale) as u16);
    let tr = hayro::kurbo::Affine::scale(scale)
        * hayro::hayro_interpret::util::TransformExt::to_kurbo(&p.initial_transform(true));
    let run = |cache: &RenderCache| {
        let t = Instant::now();
        let mut ctx = RenderContext::new(w, h);
        render_into(
            p,
            cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &mut ctx,
            tr,
        );
        ctx.flush();
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        let pm = hayro::vello_cpu::PixmapMut::new(w, h, &mut buf).unwrap();
        ctx.render_with(pm, &mut Default::default(), Default::default());
        t.elapsed().as_secs_f64() * 1e3
    };
    let fresh: Vec<f64> = (0..5).map(|_| run(&RenderCache::new())).collect();
    let shared_cache = RenderCache::new();
    let shared: Vec<f64> = (0..5).map(|_| run(&shared_cache)).collect();
    println!(
        "fresh  {:?}\nshared {:?}",
        fresh.iter().map(|x| format!("{x:.2}")).collect::<Vec<_>>(),
        shared.iter().map(|x| format!("{x:.2}")).collect::<Vec<_>>()
    );
}
