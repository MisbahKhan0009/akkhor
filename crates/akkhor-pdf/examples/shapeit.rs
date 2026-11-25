//! Shapes text with a font file and prints glyph ids (debug helper).
use harfrust::font::{Blob, Font};
use harfrust::{Buffer, Direction, ShapeOptions, ShapePlan, ShaperFont, shape};
fn main() {
    let mut a = std::env::args().skip(1);
    let data = std::fs::read(a.next().unwrap()).unwrap();
    let font = Font::new(Blob::from(data), 0).unwrap();
    let shaper = ShaperFont::new(&font);
    let script = if a.next().as_deref() == Some("deva") {
        harfrust::Script::DEVANAGARI
    } else {
        harfrust::Script::BENGALI
    };
    let plan = ShapePlan::new(&font, Direction::LeftToRight, Some(script), None, &[]);
    for t in a {
        let mut b = Buffer::new();
        b.push_str(&t);
        b.set_direction(Direction::LeftToRight);
        b.set_script(Some(script));
        shape(&shaper, &mut b, ShapeOptions::new().plan(Some(&plan))).unwrap();
        println!(
            "{t}: {:?}",
            b.glyph_infos()
                .iter()
                .map(|g| (g.glyph_id, g.cluster))
                .collect::<Vec<_>>()
        );
    }
}
