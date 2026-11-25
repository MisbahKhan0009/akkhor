//! Chosen pages of a PDF as a new PDF (the `write` switch).
//!
//! Each page is copied with everything it uses (content streams, fonts,
//! images, colour spaces, annotations) by following references from the
//! page dictionary; objects get new numbers. Streams are copied as they
//! are, still compressed, so text stays selectable and the copy is about
//! the size of what the pages use. Nothing else of the document comes
//! along: no outline, no other pages, no form fields, no structure tree.
//!
//! Kept small on purpose: the output is a classic cross-reference table
//! with one object per number, which every reader opens.

use std::collections::HashMap;
use std::io::Write;

use akkhor_core::{Error, Result};
use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, MaybeRef, Object, ObjectIdentifier};
use hayro_syntax::xref::XRef;

/// The most objects one copy may hold. A page needs a few dozen to a few
/// thousand; this only stops a hostile file from making the copy endless.
const MAX_OBJECTS: usize = 2_000_000;
/// Nesting deeper than this inside one object is written as `null`.
const MAX_DEPTH: usize = 64;
/// Page attributes a page takes from its ancestors when it lacks them.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// Page entries not copied: the tree, article beads, the structure tree,
/// and annotations (copied on their own terms, see `annotations`).
const PAGE_DROPPED: [&[u8]; 5] = [b"Type", b"Parent", b"B", b"StructParents", b"Annots"];
/// Annotation entries that point into the rest of the document.
const ANNOT_DROPPED: [&[u8]; 4] = [b"P", b"Parent", b"Popup", b"IRT"];
/// Annotations not copied: links lead to pages that are not in the copy,
/// and pop-ups belong to annotations whose link to them is dropped.
const ANNOT_SKIPPED: [&[u8]; 2] = [b"Link", b"Popup"];
/// Field entries a form widget takes from its parent fields. A copied
/// widget is made a field of its own (its parents may hold widgets on
/// pages that are not copied), so these are written into it.
const FIELD_INHERITED: [&[u8]; 8] = [b"FT", b"V", b"DV", b"Ff", b"DA", b"Q", b"Opt", b"MaxLen"];
/// Form entries copied from the document's form.
const FORM_KEPT: [&[u8]; 4] = [b"DA", b"DR", b"Q", b"NeedAppearances"];

/// What a queued object is, for the entries it keeps.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Plain,
    Annotation,
    Widget,
}

/// The pages (0-based, in the given order, repeats allowed) as a new PDF.
pub(crate) fn pages(pdf: &Pdf, chosen: &[usize]) -> Result<Vec<u8>> {
    let xref = pdf.xref();
    if xref.is_encrypted() {
        return Err(Error::Protected);
    }
    let all = pdf.pages();
    if chosen.is_empty() || chosen.iter().any(|&i| i >= all.len()) {
        return Err(Error::OutOfRange);
    }
    let mut w = Writer::new(xref);
    // 1: catalogue, 2: page tree, 3..: the pages, in order.
    w.next = 3 + chosen.len() as u32;
    let page_numbers: Vec<u32> = (0..chosen.len() as u32).map(|i| 3 + i).collect();
    for (&index, &number) in chosen.iter().zip(&page_numbers) {
        if let Some(id) = all.get(index).and_then(|p| p.raw().obj_id()) {
            w.map.entry(key(id)).or_insert(number);
        }
    }
    for (&index, &number) in chosen.iter().zip(&page_numbers) {
        let page = all.get(index).ok_or(Error::OutOfRange)?;
        w.page(number, page.raw())?;
        w.drain()?;
    }

    w.begin(2);
    w.out.extend_from_slice(b"<< /Type /Pages /Kids [");
    for n in &page_numbers {
        write!(w.out, " {n} 0 R").ok();
    }
    write!(w.out, " ] /Count {} >>", page_numbers.len()).ok();
    w.end();

    // The catalogue: optional content (so hidden layers stay hidden) and,
    // when form widgets were copied, a form listing them.
    let catalog = xref.get::<Dict<'_>>(xref.root_id());
    let mut extra = Vec::new();
    std::mem::swap(&mut w.out, &mut extra);
    if let Some(v) = catalog
        .as_ref()
        .and_then(|c| c.get_raw::<Object<'_>>(b"OCProperties"))
    {
        w.out.extend_from_slice(b" /OCProperties ");
        w.value(v, 1);
    }
    if !w.widgets.is_empty() {
        w.out.extend_from_slice(b" /AcroForm << /Fields [");
        for n in w.widgets.clone() {
            write!(w.out, " {n} 0 R").ok();
        }
        w.out.extend_from_slice(b" ]");
        if let Some(form) = catalog
            .as_ref()
            .and_then(|c| c.get::<Dict<'_>>(b"AcroForm"))
        {
            for name in FORM_KEPT {
                if let Some(v) = form.get_raw::<Object<'_>>(name) {
                    w.out.push(b' ');
                    name_bytes(&mut w.out, name);
                    w.out.push(b' ');
                    w.value(v, 2);
                }
            }
        }
        w.out.extend_from_slice(b" >>");
    }
    std::mem::swap(&mut w.out, &mut extra);
    w.drain()?;
    w.begin(1);
    w.out.extend_from_slice(b"<< /Type /Catalog /Pages 2 0 R");
    w.out.extend_from_slice(&extra);
    w.out.extend_from_slice(b" >>");
    w.end();
    Ok(w.finish())
}

fn key(id: ObjectIdentifier) -> (u32, i32) {
    let r: hayro_syntax::object::ObjRef = id.into();
    (r.obj_number, r.gen_number)
}

struct Writer<'a> {
    xref: &'a XRef,
    /// Old object (number, generation) to new number.
    map: HashMap<(u32, i32), u32>,
    /// Objects given a number but not written yet.
    queue: Vec<((u32, i32), u32, Kind)>,
    /// Copied form widgets, listed in the copy's form.
    widgets: Vec<u32>,
    next: u32,
    out: Vec<u8>,
    /// New number to byte offset; 0 for numbers never written.
    offsets: Vec<usize>,
}

impl<'a> Writer<'a> {
    fn new(xref: &'a XRef) -> Self {
        let mut out = Vec::with_capacity(1 << 16);
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        Self {
            xref,
            map: HashMap::new(),
            queue: Vec::new(),
            widgets: Vec::new(),
            next: 1,
            out,
            offsets: Vec::new(),
        }
    }

    fn begin(&mut self, number: u32) {
        let n = number as usize;
        if self.offsets.len() <= n {
            self.offsets.resize(n + 1, 0);
        }
        self.offsets[n] = self.out.len();
        writeln!(self.out, "{number} 0 obj").ok();
    }

    fn end(&mut self) {
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// A page dictionary, with the inherited attributes written in and
    /// the link to the page tree replaced.
    fn page(&mut self, number: u32, dict: &Dict<'a>) -> Result<()> {
        let mut body = Vec::new();
        std::mem::swap(&mut self.out, &mut body);
        self.out.extend_from_slice(b"<< /Type /Page /Parent 2 0 R");
        for name in INHERITED {
            if let Some(v) = inherited(dict, name) {
                self.out.push(b' ');
                name_bytes(&mut self.out, name);
                self.out.push(b' ');
                self.value(v, 1);
            }
        }
        for (name, v) in dict.entries() {
            if PAGE_DROPPED.contains(&name.as_ref()) || INHERITED.contains(&name.as_ref()) {
                continue;
            }
            self.out.push(b' ');
            name_bytes(&mut self.out, &name);
            self.out.push(b' ');
            self.value(v, 1);
        }
        self.annotations(dict);
        self.out.extend_from_slice(b" >>");
        std::mem::swap(&mut self.out, &mut body);
        self.begin(number);
        self.out.extend_from_slice(&body);
        self.end();
        Ok(())
    }

    /// The page's annotations that stand on their own: highlights, notes,
    /// stamps, drawn form fields (their look, not their field).
    fn annotations(&mut self, page: &Dict<'a>) {
        let Some(list) = page.get::<hayro_syntax::object::Array<'_>>(b"Annots") else {
            return;
        };
        let mut items = Vec::new();
        for item in list.raw_iter() {
            let (dict, id) = match item {
                MaybeRef::Ref(r) => {
                    let id: ObjectIdentifier = r.into();
                    (self.xref.get::<Dict<'_>>(id), Some(id))
                }
                MaybeRef::NotRef(o) => (o.into_dict(), None),
            };
            let Some(dict) = dict else { continue };
            let subtype = dict.get::<hayro_syntax::object::Name<'_>>(b"Subtype");
            if subtype
                .as_ref()
                .is_some_and(|s| ANNOT_SKIPPED.contains(&s.as_ref()))
            {
                continue;
            }
            let kind = if subtype.is_some_and(|s| s.as_ref() == b"Widget") {
                Kind::Widget
            } else {
                Kind::Annotation
            };
            match id {
                Some(id) => {
                    let fresh = !self.map.contains_key(&key(id));
                    let number = self.number_for(key(id), kind);
                    if kind == Kind::Widget && fresh {
                        self.widgets.push(number);
                    }
                    items.push(format!("{number} 0 R").into_bytes());
                }
                None => {
                    let mut out = Vec::new();
                    std::mem::swap(&mut self.out, &mut out);
                    self.annotation(&dict, kind, 2);
                    std::mem::swap(&mut self.out, &mut out);
                    items.push(out);
                }
            }
        }
        if items.is_empty() {
            return;
        }
        self.out.extend_from_slice(b" /Annots [");
        for item in items {
            self.out.push(b' ');
            self.out.extend_from_slice(&item);
        }
        self.out.extend_from_slice(b" ]");
    }

    fn number_for(&mut self, old: (u32, i32), kind: Kind) -> u32 {
        if let Some(&n) = self.map.get(&old) {
            return n;
        }
        let n = self.next;
        self.next += 1;
        self.map.insert(old, n);
        self.queue.push((old, n, kind));
        n
    }

    /// An annotation without its links into the rest of the document; a
    /// form widget also gets the field entries of its parents.
    fn annotation(&mut self, d: &Dict<'a>, kind: Kind, depth: usize) {
        if kind != Kind::Widget || !has(d, b"Parent") {
            self.dict(d, &ANNOT_DROPPED, depth);
            return;
        }
        // The full name is written below, from the widget and its parents.
        let dropped: Vec<&[u8]> = ANNOT_DROPPED.iter().copied().chain([&b"T"[..]]).collect();
        self.dict(d, &dropped, depth);
        // Reopen the dictionary.
        self.out.truncate(self.out.len() - 3);
        let mut names = Vec::new();
        let mut node = Some(d.clone());
        for _ in 0..32 {
            let Some(n) = node else { break };
            if let Some(t) = n.get::<hayro_syntax::object::String<'_>>(b"T") {
                names.push(t.as_bytes().to_vec());
            }
            node = n.get::<Dict<'_>>(b"Parent");
        }
        if !names.is_empty() {
            names.reverse();
            self.out.extend_from_slice(b" /T <");
            for b in names.join(&b'.') {
                write!(self.out, "{b:02X}").ok();
            }
            self.out.push(b'>');
        }
        for name in FIELD_INHERITED {
            if has(d, name) {
                continue;
            }
            let mut node = d.get::<Dict<'_>>(b"Parent");
            for _ in 0..32 {
                let Some(n) = node else { break };
                if let Some(v) = n.get_raw::<Object<'_>>(name).filter(|v| !is_null(v)) {
                    self.out.push(b' ');
                    name_bytes(&mut self.out, name);
                    self.out.push(b' ');
                    self.value(v, depth + 1);
                    break;
                }
                node = n.get::<Dict<'_>>(b"Parent");
            }
        }
        self.out.extend_from_slice(b" >>");
    }

    /// Writes every object reached so far.
    fn drain(&mut self) -> Result<()> {
        while let Some((old, number, kind)) = self.queue.pop() {
            if self.map.len() > MAX_OBJECTS {
                return Err(Error::malformed("the pages use too many objects"));
            }
            let id = ObjectIdentifier::new(old.0, old.1);
            let object = self.xref.get::<Object<'_>>(id);
            self.begin(number);
            match object {
                Some(Object::Stream(s)) => {
                    let data = s.raw_data();
                    self.dict(s.dict(), &[b"Length"], 1);
                    // Reopen the dictionary to add the true length.
                    self.out.truncate(self.out.len() - 3);
                    write!(self.out, " /Length {} >>\nstream\n", data.len()).ok();
                    self.out.extend_from_slice(&data);
                    self.out.extend_from_slice(b"\nendstream");
                }
                Some(Object::Dict(d)) if kind != Kind::Plain => self.annotation(&d, kind, 1),
                Some(o) => self.object(o, 1),
                None => self.out.extend_from_slice(b"null"),
            }
            self.end();
        }
        Ok(())
    }

    fn value(&mut self, v: MaybeRef<Object<'a>>, depth: usize) {
        match v {
            MaybeRef::NotRef(o) => self.object(o, depth),
            MaybeRef::Ref(r) => {
                let old = (r.obj_number, r.gen_number);
                if let Some(&n) = self.map.get(&old) {
                    write!(self.out, "{n} 0 R").ok();
                    return;
                }
                // A page or page-tree node that is not being copied (a
                // link target, an annotation's page): leave it out, or
                // the whole document would come along.
                let target = self.xref.get::<Object<'_>>(r.into());
                let skip = match &target {
                    None => true,
                    Some(Object::Dict(d)) => d
                        .get::<hayro_syntax::object::Name<'_>>(b"Type")
                        .is_some_and(|t| t.as_ref() == b"Page" || t.as_ref() == b"Pages"),
                    Some(_) => false,
                };
                if skip {
                    self.out.extend_from_slice(b"null");
                } else {
                    let n = self.number_for(old, Kind::Plain);
                    write!(self.out, "{n} 0 R").ok();
                }
            }
        }
    }

    fn object(&mut self, o: Object<'a>, depth: usize) {
        if depth > MAX_DEPTH {
            self.out.extend_from_slice(b"null");
            return;
        }
        match o {
            Object::Null(_) => self.out.extend_from_slice(b"null"),
            Object::Boolean(b) => self
                .out
                .extend_from_slice(if b { b"true" } else { b"false" }),
            Object::Number(n) => number(&mut self.out, n.as_f64()),
            Object::String(s) => {
                self.out.push(b'<');
                for b in s.as_bytes() {
                    write!(self.out, "{b:02X}").ok();
                }
                self.out.push(b'>');
            }
            Object::Name(n) => name_bytes(&mut self.out, &n),
            Object::Array(a) => {
                self.out.push(b'[');
                for item in a.raw_iter() {
                    self.out.push(b' ');
                    self.value(item, depth + 1);
                }
                self.out.extend_from_slice(b" ]");
            }
            Object::Dict(d) => self.dict(&d, &[], depth),
            // A stream can only be an indirect object; one met inline is
            // damage.
            Object::Stream(_) => self.out.extend_from_slice(b"null"),
        }
    }

    fn dict(&mut self, d: &Dict<'a>, dropped: &[&[u8]], depth: usize) {
        self.out.extend_from_slice(b"<<");
        for (name, v) in d.entries() {
            // A null value is the same as no entry.
            if dropped.contains(&name.as_ref()) || is_null(&v) {
                continue;
            }
            self.out.push(b' ');
            name_bytes(&mut self.out, &name);
            self.out.push(b' ');
            self.value(v, depth + 1);
        }
        self.out.extend_from_slice(b" >>");
    }

    fn finish(mut self) -> Vec<u8> {
        let size = self.offsets.len().max(self.next as usize);
        self.offsets.resize(size, 0);
        let start = self.out.len();
        write!(self.out, "xref\n0 {size}\n0000000000 65535 f\r\n").ok();
        for &o in &self.offsets[1..] {
            if o == 0 {
                self.out.extend_from_slice(b"0000000000 00000 f\r\n");
            } else {
                write!(self.out, "{o:010} 00000 n\r\n").ok();
            }
        }
        write!(
            self.out,
            "trailer\n<< /Size {size} /Root 1 0 R /Info << /Producer (Akkhor) >> >>\nstartxref\n{start}\n%%EOF\n"
        )
        .ok();
        self.out
    }
}

/// Whether a dictionary has an entry that is not null.
fn has(d: &Dict<'_>, name: &[u8]) -> bool {
    d.get_raw::<Object<'_>>(name).is_some_and(|v| !is_null(&v))
}

fn is_null(v: &MaybeRef<Object<'_>>) -> bool {
    matches!(v, MaybeRef::NotRef(Object::Null(_)))
}

/// A page attribute from the page or, if it lacks it, its nearest
/// ancestor that has it.
fn inherited<'a>(page: &Dict<'a>, name: &[u8]) -> Option<MaybeRef<Object<'a>>> {
    let mut node = page.clone();
    for _ in 0..64 {
        if let Some(v) = node.get_raw::<Object<'_>>(name) {
            return Some(v);
        }
        node = node.get::<Dict<'_>>(b"Parent")?;
    }
    None
}

fn number(out: &mut Vec<u8>, v: f64) {
    if !v.is_finite() {
        out.push(b'0');
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        write!(out, "{}", v as i64).ok();
    } else {
        let s = format!("{v:.6}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        out.extend_from_slice(s.as_bytes());
    }
}

/// A name, with `#xx` for bytes a name cannot hold as they are.
fn name_bytes(out: &mut Vec<u8>, name: &[u8]) {
    out.push(b'/');
    for &b in name {
        let plain = (0x21..=0x7E).contains(&b) && !b"#()<>[]{}/%".contains(&b);
        if plain {
            out.push(b);
        } else {
            write!(out, "#{b:02X}").ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_names_are_written_as_pdf_reads_them() {
        let mut out = Vec::new();
        for v in [0.0, -3.0, 595.0, 0.5, 1.0 / 3.0, -0.000_000_1] {
            number(&mut out, v);
            out.push(b' ');
        }
        assert_eq!(out, b"0 -3 595 0.5 0.333333 -0 ");
        let mut out = Vec::new();
        name_bytes(&mut out, b"A B#(c)");
        assert_eq!(out, b"/A#20B#23#28c#29");
    }
}
