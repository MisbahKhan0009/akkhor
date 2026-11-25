//! The table of contents of a PDF (its "outlines", or bookmarks).
//!
//! Read on demand: a linked list of items, each with a title and a
//! destination that names a page by its object. Destinations come as an
//! array, a name looked up in the document's name tree, or a `GoTo`
//! action. Files can loop or lie, so every walk is bounded.

use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, String as PdfString,
};

use crate::glyphs::text_string;

/// One entry of the table of contents.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    /// Nesting level, 0 for top-level entries.
    pub level: usize,
    /// Target page index, if the entry points at a page of this document.
    pub page: Option<usize>,
}

/// Entries beyond this many are ignored (a looping or hostile file).
const MAX_ITEMS: usize = 10_000;
const MAX_DEPTH: usize = 32;

/// Page object → page index, read from the page tree without parsing the
/// pages (a flat node's kids are its pages).
pub(crate) fn page_index(pdf: &Pdf) -> HashMap<ObjectIdentifier, usize> {
    let xref = pdf.xref();
    let mut out = HashMap::new();
    let Some(root) = xref.get::<Dict<'_>>(xref.root_id()) else {
        return out;
    };
    let Some(pages) = root.get::<Dict<'_>>(b"Pages") else {
        return out;
    };
    let mut seen = HashSet::new();
    walk_tree(pdf, &pages, &mut out, &mut seen, 0);
    out
}

fn walk_tree(
    pdf: &Pdf,
    node: &Dict<'_>,
    out: &mut HashMap<ObjectIdentifier, usize>,
    seen: &mut HashSet<ObjectIdentifier>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(kids) = node.get::<Array<'_>>(b"Kids") else {
        return;
    };
    let refs: Vec<MaybeRef<Object<'_>>> = kids.raw_iter().collect();
    let flat = node.get::<i64>(b"Count") == Some(refs.len() as i64);
    for kid in refs {
        let MaybeRef::Ref(r) = kid else { continue };
        let id: ObjectIdentifier = r.into();
        if !seen.insert(id) {
            continue;
        }
        if flat {
            let next = out.len();
            out.insert(id, next);
            continue;
        }
        let Some(dict) = pdf.xref().get::<Dict<'_>>(id) else {
            continue;
        };
        if dict.get::<Name<'_>>(b"Type").as_deref() == Some(b"Pages".as_slice()) {
            walk_tree(pdf, &dict, out, seen, depth + 1);
        } else {
            let next = out.len();
            out.insert(id, next);
        }
    }
}

/// The document's table of contents, in reading order.
pub(crate) fn read(pdf: &Pdf) -> Vec<OutlineItem> {
    let xref = pdf.xref();
    let Some(root) = xref.get::<Dict<'_>>(xref.root_id()) else {
        return Vec::new();
    };
    let Some(outlines) = root.get::<Dict<'_>>(b"Outlines") else {
        return Vec::new();
    };
    let pages = page_index(pdf);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(first) = outlines.get::<Dict<'_>>(b"First") {
        walk_items(&root, &pages, first, 0, &mut out, &mut seen);
    }
    out
}

fn walk_items(
    root: &Dict<'_>,
    pages: &HashMap<ObjectIdentifier, usize>,
    first: Dict<'_>,
    level: usize,
    out: &mut Vec<OutlineItem>,
    seen: &mut HashSet<ObjectIdentifier>,
) {
    let mut item = Some(first);
    while let Some(dict) = item {
        if out.len() >= MAX_ITEMS || level > MAX_DEPTH {
            return;
        }
        if let Some(id) = dict.obj_id()
            && !seen.insert(id)
        {
            return; // a loop
        }
        let title = dict
            .get::<PdfString<'_>>(b"Title")
            .map(|t| clean_title(&text_string(t.as_bytes())))
            .unwrap_or_default();
        let page = destination(root, &dict).and_then(|id| pages.get(&id).copied());
        out.push(OutlineItem { title, level, page });
        if let Some(child) = dict.get::<Dict<'_>>(b"First") {
            walk_items(root, pages, child, level + 1, out, seen);
        }
        item = dict.get::<Dict<'_>>(b"Next");
    }
}

fn clean_title(t: &str) -> String {
    t.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The page object an outline item points at.
fn destination(root: &Dict<'_>, item: &Dict<'_>) -> Option<ObjectIdentifier> {
    if let Some(id) = dest_page(root, item, b"Dest") {
        return Some(id);
    }
    let action = item.get::<Dict<'_>>(b"A")?;
    if action.get::<Name<'_>>(b"S").as_deref() != Some(b"GoTo".as_slice()) {
        return None;
    }
    dest_page(root, &action, b"D")
}

/// Resolves `key` of `dict` as a destination: an explicit array, or a name
/// or string looked up among the named destinations.
fn dest_page(root: &Dict<'_>, dict: &Dict<'_>, key: &[u8]) -> Option<ObjectIdentifier> {
    if let Some(array) = dict.get::<Array<'_>>(key) {
        return array_page(&array);
    }
    let name: Vec<u8> = if let Some(n) = dict.get::<Name<'_>>(key) {
        n.as_ref().to_vec()
    } else {
        dict.get::<PdfString<'_>>(key)?.as_bytes().to_vec()
    };
    named(root, &name)
}

fn array_page(array: &Array<'_>) -> Option<ObjectIdentifier> {
    match array.raw_iter().next()? {
        MaybeRef::Ref(r) => Some(r.into()),
        MaybeRef::NotRef(_) => None, // a page number: another document's
    }
}

/// A named destination: the catalog's `/Dests` dictionary (PDF 1.1) or the
/// `/Names /Dests` name tree.
fn named(root: &Dict<'_>, name: &[u8]) -> Option<ObjectIdentifier> {
    if let Some(dests) = root.get::<Dict<'_>>(b"Dests") {
        for (key, _) in dests.entries() {
            if key.as_ref() == name {
                return dest_value(&dests, key.as_ref());
            }
        }
    }
    let tree = root.get::<Dict<'_>>(b"Names")?.get::<Dict<'_>>(b"Dests")?;
    let mut budget = 10_000;
    find_in_name_tree(&tree, name, 0, &mut budget)
}

/// A destination value: an array, or a dictionary holding one under `/D`.
fn dest_value(dict: &Dict<'_>, key: &[u8]) -> Option<ObjectIdentifier> {
    if let Some(array) = dict.get::<Array<'_>>(key) {
        return array_page(&array);
    }
    let d = dict.get::<Dict<'_>>(key)?;
    array_page(&d.get::<Array<'_>>(b"D")?)
}

fn find_in_name_tree(
    node: &Dict<'_>,
    name: &[u8],
    depth: usize,
    budget: &mut usize,
) -> Option<ObjectIdentifier> {
    if depth > MAX_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(names) = node.get::<Array<'_>>(b"Names") {
        let items: Vec<Object<'_>> = names.iter::<Object<'_>>().collect();
        for pair in items.chunks(2) {
            let [key, value] = pair else { continue };
            let key_bytes = key.clone().into_string().map(|s| s.as_bytes().to_vec());
            if key_bytes.as_deref() == Some(name) {
                return match value.clone() {
                    Object::Array(a) => array_page(&a),
                    Object::Dict(d) => array_page(&d.get::<Array<'_>>(b"D")?),
                    _ => None,
                };
            }
        }
    }
    if let Some(kids) = node.get::<Array<'_>>(b"Kids") {
        for kid in kids.iter::<Dict<'_>>() {
            if let Some(found) = find_in_name_tree(&kid, name, depth + 1, budget) {
                return Some(found);
            }
        }
    }
    None
}
