//! Shared helpers for the fuzz targets.

use akkhor_flow::{Document, FontSet, LayoutOptions, paginate};

/// A stored (uncompressed) zip holding these files.
pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, body) in files {
        let off = out.len() as u32;
        let len = (body.len() as u32).to_le_bytes();
        out.extend_from_slice(b"PK\x03\x04\x14\0\0\0\0\0\0\0\0\0\0\0\0\0");
        out.extend_from_slice(&len);
        out.extend_from_slice(&len);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        central.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\0\0\0\0\0\0\0\0\0\0\0");
        central.extend_from_slice(&len);
        central.extend_from_slice(&len);
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&off.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// An EPUB with one chapter and one style sheet.
pub fn epub(chapter: &[u8], css: &[u8]) -> Vec<u8> {
    let container = br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
    let opf = br#"<package version="3.0"><metadata><dc:title>t</dc:title></metadata>
        <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
        <item id="s" href="s.css" media-type="text/css"/></manifest>
        <spine><itemref idref="c1"/></spine></package>"#;
    zip(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container),
        ("OEBPS/content.opf", opf),
        ("OEBPS/ch1.xhtml", chapter),
        ("OEBPS/s.css", css),
    ])
}

/// Lays a document out on small pages, at most a few of them.
pub fn lay_out(doc: &Document) {
    thread_local! {
        static FONTS: FontSet = FontSet::bundled();
    }
    let opts = LayoutOptions {
        width: 360.0,
        height: 480.0,
        ..LayoutOptions::default()
    };
    FONTS.with(|fonts| {
        let pages = paginate(doc, &opts, fonts);
        std::hint::black_box(pages.len());
    });
}

/// Reads the pictures a chapter names: header size, and a small decode.
pub fn pictures(doc: &Document, images: &dyn akkhor_flow::ImageProvider) {
    for src in doc.blocks.iter().filter_map(|b| b.image.as_deref()).take(4) {
        let _ = images.size(src);
        if let Some(img) = images.decoded(src, 64, 64) {
            std::hint::black_box(img.rgba.len());
        }
    }
}
