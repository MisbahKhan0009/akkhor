//! Reference-font tables kept on disk between runs.
//!
//! Building the tables for a reference font (shaping its whole repertoire
//! backwards, hashing and picturing every glyph) takes 30 to 150 ms a font
//! on a phone, once per process. With a cache folder set
//! ([`set_cache_dir`]), each table is written once and read back in a few
//! milliseconds. A file names the font by a hash of its bytes and the
//! engine by a fingerprint of the code that builds tables, so a changed
//! font or a newer engine never reads an old table; a damaged file is
//! rebuilt.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::recover::{GlyphText, Recovery, Role};
use crate::shape::Shape;

const MAGIC: &[u8; 8] = b"AKKHORT1";
/// Fingerprint of the table-building code (build.rs).
const ENGINE: &str = env!("AKKHOR_TABLES");

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where reference-font tables are kept (an app's cache folder). Only the
/// first call counts.
pub fn set_cache_dir(dir: impl Into<PathBuf>) -> bool {
    let dir = dir.into();
    if std::fs::create_dir_all(&dir).is_err() || DIR.set(dir.clone()).is_err() {
        return false;
    }
    prune(&dir);
    true
}

/// Removes tables an older engine wrote, and half-written files.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let current = format!("-{ENGINE}.bin");
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours =
            name.starts_with("akkhor-") && (name.ends_with(".bin") || name.ends_with(".part"));
        if ours && !name.ends_with(&current) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// FNV-1a over the font's length, its first and its last 64 KB: stable
/// across builds and platforms. The start holds the table directory with
/// every table's checksum, so another font, or another version of one,
/// differs there; hashing whole files cost a few ms a launch on a phone.
pub(crate) fn font_key(data: &[u8]) -> u64 {
    const PART: usize = 64 << 10;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(&(data.len() as u64).to_le_bytes());
    if data.len() <= 2 * PART {
        eat(data);
    } else {
        eat(&data[..PART]);
        eat(&data[data.len() - PART..]);
    }
    h
}

fn path(key: u64, kind: &str) -> Option<PathBuf> {
    Some(
        DIR.get()?
            .join(format!("akkhor-{key:016x}-{kind}-{ENGINE}.bin")),
    )
}

fn load(key: u64, kind: &str) -> Option<Vec<u8>> {
    let b = std::fs::read(path(key, kind)?).ok()?;
    b.strip_prefix(MAGIC.as_slice()).map(<[u8]>::to_vec)
}

fn save(key: u64, kind: &str, body: &[u8]) {
    let Some(p) = path(key, kind) else { return };
    let tmp = p.with_extension(format!("{}.part", std::process::id()));
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(body);
    // Written beside, then moved in: a reader never sees half a file.
    if std::fs::write(&tmp, &out).is_ok() && std::fs::rename(&tmp, &p).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// A stored table, or `build` run and its result stored.
pub(crate) fn cached<T>(
    key: u64,
    kind: &str,
    build: impl FnOnce() -> T,
    write: impl Fn(&T, &mut W),
    read: impl Fn(&mut R) -> Option<T>,
) -> T {
    if DIR.get().is_none() {
        return build();
    }
    if let Some(body) = load(key, kind)
        && let Some(v) = read(&mut R(&body))
    {
        return v;
    }
    let v = build();
    let mut w = W(Vec::new());
    write(&v, &mut w);
    save(key, kind, &w.0);
    v
}

pub(crate) struct W(Vec<u8>);

impl W {
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
}

pub(crate) struct R<'a>(&'a [u8]);

impl R<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    pub fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    /// A count no larger than the bytes left could hold.
    pub fn count(&mut self, each: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n.saturating_mul(each) <= self.0.len()).then_some(n)
    }
    pub fn done(&self) -> bool {
        self.0.is_empty()
    }
}

fn role_code(r: Role) -> u32 {
    match r {
        Role::Base => 0,
        Role::PreBase => 1,
        Role::Reph => 2,
        Role::PreBaseReph => 3,
    }
}

fn role(c: u32) -> Option<Role> {
    Some(match c {
        0 => Role::Base,
        1 => Role::PreBase,
        2 => Role::Reph,
        3 => Role::PreBaseReph,
        _ => return None,
    })
}

pub(crate) fn write_recovery(r: &Option<Recovery>, w: &mut W) {
    let Some(r) = r else {
        w.u32(0);
        return;
    };
    w.u32(1);
    let mut map: Vec<_> = r.map.iter().collect();
    map.sort_by_key(|(k, _)| **k);
    w.u32(map.len() as u32);
    for (gid, t) in map {
        w.u32(*gid);
        w.u32(role_code(t.role));
        w.str(&t.text);
    }
    let mut seqs: Vec<_> = r.sequences.iter().collect();
    seqs.sort();
    w.u32(seqs.len() as u32);
    for (k, v) in seqs {
        w.u32(k.len() as u32);
        for g in k {
            w.u32(*g);
        }
        w.str(v);
    }
    w.u32(r.max_sequence as u32);
    for set in [&r.weak, &r.blank] {
        let mut v: Vec<_> = set.iter().copied().collect();
        v.sort_unstable();
        w.u32(v.len() as u32);
        for g in v {
            w.u32(g);
        }
    }
    w.u32(u32::from(r.complete) | (u32::from(r.indic) << 1));
}

pub(crate) fn read_recovery(r: &mut R) -> Option<Option<Recovery>> {
    if r.u32()? == 0 {
        return r.done().then_some(None);
    }
    let mut out = Recovery::default();
    for _ in 0..r.count(12)? {
        let gid = r.u32()?;
        let role = role(r.u32()?)?;
        let text = r.str()?;
        out.map.insert(gid, GlyphText { text, role });
    }
    for _ in 0..r.count(8)? {
        let n = r.count(4)?;
        let key = (0..n).map(|_| r.u32()).collect::<Option<Vec<u32>>>()?;
        out.sequences.insert(key, r.str()?);
    }
    out.max_sequence = r.u32()? as usize;
    let mut sets: [HashSet<u32>; 2] = Default::default();
    for set in &mut sets {
        for _ in 0..r.count(4)? {
            set.insert(r.u32()?);
        }
    }
    [out.weak, out.blank] = sets;
    let flags = r.u32()?;
    out.complete = flags & 1 != 0;
    out.indic = flags & 2 != 0;
    r.done().then_some(Some(out))
}

pub(crate) fn write_shapes(m: &HashMap<u64, u32>, w: &mut W) {
    let mut v: Vec<_> = m.iter().collect();
    v.sort();
    w.u32(v.len() as u32);
    for (h, g) in v {
        w.u64(*h);
        w.u32(*g);
    }
}

pub(crate) fn read_shapes(r: &mut R) -> Option<HashMap<u64, u32>> {
    let mut m = HashMap::new();
    for _ in 0..r.count(12)? {
        let h = r.u64()?;
        m.insert(h, r.u32()?);
    }
    r.done().then_some(m)
}

pub(crate) fn write_pictures(v: &[(u32, f32, Shape)], w: &mut W) {
    w.u32(v.len() as u32);
    for (g, adv, shape) in v {
        w.u32(*g);
        w.f32(*adv);
        for row in shape.0 {
            w.u64(row);
        }
    }
}

pub(crate) fn read_pictures(r: &mut R) -> Option<Vec<(u32, f32, Shape)>> {
    let rows = Shape::default().0.len();
    let n = r.count(8 + rows * 8)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let g = r.u32()?;
        let adv = r.f32()?;
        let mut s = Shape::default();
        for row in s.0.iter_mut() {
            *row = r.u64()?;
        }
        out.push((g, adv, s));
    }
    r.done().then_some(out)
}

#[cfg(all(test, feature = "bundled-fonts"))]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn font_keys_tell_fonts_apart() {
        let fonts = akkhor_flow::fonts::BUNDLED;
        let keys: HashSet<u64> = fonts.iter().map(|(_, b)| font_key(b)).collect();
        assert_eq!(keys.len(), fonts.len());
        // A changed byte at either end, or a different length, is another font.
        let base: Vec<u8> = fonts[0].1.to_vec();
        let k = font_key(&base);
        for at in [0, 40, base.len() - 1] {
            let mut b = base.clone();
            b[at] ^= 1;
            assert_ne!(font_key(&b), k, "byte {at}");
        }
        assert_ne!(font_key(&base[..base.len() - 1]), k);
    }

    #[test]
    fn tables_read_back_as_built() {
        let (_, bytes) = akkhor_flow::fonts::BUNDLED
            .iter()
            .find(|(s, _)| s.contains(&akkhor_flow::script::Script::Bengali))
            .unwrap();
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(*bytes);
        for deep in [false, true] {
            let built = crate::recover::build_with(&data, true, deep);
            let mut w = W(Vec::new());
            write_recovery(&built, &mut w);
            let back = read_recovery(&mut R(&w.0)).unwrap().unwrap();
            let built = built.unwrap();
            assert!(!built.map.is_empty());
            assert_eq!(back.map, built.map);
            assert_eq!(back.sequences, built.sequences);
            assert_eq!(back.max_sequence, built.max_sequence);
            assert_eq!(back.weak, built.weak);
            assert_eq!(back.blank, built.blank);
            assert_eq!((back.complete, back.indic), (built.complete, built.indic));
            // Any cut makes the file unreadable rather than wrong.
            for cut in [0, 1, w.0.len() / 2, w.0.len() - 1] {
                assert!(read_recovery(&mut R(&w.0[..cut])).is_none_or(|r| r.is_none() && cut == 4));
            }
        }
        let shapes: HashMap<u64, u32> = [(1u64, 2u32), (u64::MAX, 7)].into_iter().collect();
        let mut w = W(Vec::new());
        write_shapes(&shapes, &mut w);
        assert_eq!(read_shapes(&mut R(&w.0)).unwrap(), shapes);
        let mut s = Shape::default();
        s.0[3] = 0xdead_beef;
        let pics = vec![(5u32, 512.5f32, s)];
        let mut w = W(Vec::new());
        write_pictures(&pics, &mut w);
        assert_eq!(read_pictures(&mut R(&w.0)).unwrap(), pics);
        // A count claiming more than the bytes hold is refused, not allocated.
        assert!(read_pictures(&mut R(&u32::MAX.to_le_bytes())).is_none());
    }
}
