//! EPUB 2 and 3 support for Akkhor.
//!
//! Opening an EPUB reads the zip's central directory, `container.xml` and the
//! package document (OPF): the metadata, reading order (spine) and table of
//! contents. Chapters are inflated and parsed one at a time, on demand.

pub mod zip;

use akkhor_core::{Error, Result, Source};
use akkhor_flow::Document;
use akkhor_flow::image::ImageProvider;
use akkhor_flow::markup::{Token, Tokenizer};
use akkhor_flow::model::linked_stylesheets;
use akkhor_flow::search::{Match, find_all};

use crate::zip::ZipArchive;

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TocEntry {
    pub title: String,
    /// Full path inside the archive (fragment removed).
    pub href: String,
    pub depth: u8,
}

pub struct EpubBook {
    zip: ZipArchive,
    pub metadata: Metadata,
    /// Archive paths of the chapters, in reading order.
    pub spine: Vec<String>,
    pub toc: Vec<TocEntry>,
}

impl EpubBook {
    pub fn open(source: Source) -> Result<Self> {
        source.advise_random();
        let zip = ZipArchive::open(source)?;
        if is_drm_protected(&zip) {
            return Err(Error::Protected);
        }

        let container = zip.read_string("META-INF/container.xml")?;
        let opf_path = Tokenizer::new(&container)
            .find_map(|t| match t {
                Token::Start { name, attrs, .. } if name == "rootfile" => attr(&attrs, "full-path"),
                _ => None,
            })
            .ok_or_else(|| Error::malformed("EPUB has no package document"))?;
        let opf_dir = parent_dir(&opf_path);
        let opf = zip.read_string(&opf_path)?;

        let mut metadata = Metadata::default();
        let mut manifest: Vec<(String, String, String, String)> = Vec::new(); // id, href, media, props
        let mut spine_ids: Vec<String> = Vec::new();
        let mut ncx_id: Option<String> = None;
        let mut current_meta: Option<&'static str> = None;

        for tok in Tokenizer::new(&opf) {
            match tok {
                Token::Start { name, attrs, .. } => match name.as_str() {
                    "title" => current_meta = Some("title"),
                    "creator" => current_meta = Some("creator"),
                    "language" => current_meta = Some("language"),
                    "item" => {
                        let get = |k| attr(&attrs, k).unwrap_or_default();
                        manifest.push((
                            get("id"),
                            get("href"),
                            get("media-type"),
                            get("properties"),
                        ));
                    }
                    "spine" => ncx_id = attr(&attrs, "toc"),
                    "itemref" => {
                        if attr(&attrs, "linear").as_deref() != Some("no")
                            && let Some(id) = attr(&attrs, "idref")
                        {
                            spine_ids.push(id);
                        }
                    }
                    _ => current_meta = None,
                },
                Token::Text(t) => {
                    if let Some(field) = current_meta.take() {
                        let v = t.trim().to_string();
                        if !v.is_empty() {
                            let slot = match field {
                                "title" => &mut metadata.title,
                                "creator" => &mut metadata.creator,
                                _ => &mut metadata.language,
                            };
                            slot.get_or_insert(v);
                        }
                    }
                }
                Token::End { .. } => current_meta = None,
            }
        }

        let resolve = |href: &str| join_path(&opf_dir, &percent_decode(href));
        let spine = spine_ids
            .iter()
            .filter_map(|id| manifest.iter().find(|m| &m.0 == id))
            .map(|m| resolve(&m.1))
            .collect();

        // Table of contents: EPUB 3 nav document, else EPUB 2 NCX.
        let nav = manifest
            .iter()
            .find(|m| m.3.split_whitespace().any(|p| p == "nav"));
        let ncx = ncx_id
            .and_then(|id| manifest.iter().find(|m| m.0 == id))
            .or_else(|| manifest.iter().find(|m| m.2 == "application/x-dtbncx+xml"));
        let toc = if let Some(nav) = nav {
            let path = resolve(&nav.1);
            zip.read_string(&path)
                .map(|s| parse_nav(&s, &parent_dir(&path)))
                .unwrap_or_default()
        } else if let Some(ncx) = ncx {
            let path = resolve(&ncx.1);
            zip.read_string(&path)
                .map(|s| parse_ncx(&s, &parent_dir(&path)))
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Self {
            zip,
            metadata,
            spine,
            toc,
        })
    }

    pub fn chapter_count(&self) -> usize {
        self.spine.len()
    }

    /// Inflates and parses one chapter.
    pub fn chapter(&self, index: usize) -> Result<Document> {
        let path = self.spine.get(index).ok_or(Error::OutOfRange)?;
        let html = self.zip.read_string(path)?;
        let dir = parent_dir(path);
        let css: Vec<String> = linked_stylesheets(&html)
            .iter()
            .filter_map(|href| {
                self.zip
                    .read_string(&join_path(&dir, &percent_decode(href)))
                    .ok()
            })
            .collect();
        let css_refs: Vec<&str> = css.iter().map(String::as_str).collect();
        let mut doc = Document::from_html_with_css(&html, &css_refs);
        if doc.lang.is_none() {
            doc.lang = self.metadata.language.clone();
        }
        Ok(doc)
    }

    /// Searches the whole book chapter by chapter, lazily: each chapter is
    /// only inflated and parsed when the iterator reaches it, so the caller
    /// can show the first hits at once and stop at any time.
    pub fn search<'a>(&'a self, query: &'a str) -> impl Iterator<Item = (usize, Match)> + 'a {
        (0..self.chapter_count()).flat_map(move |i| {
            self.chapter(i)
                .map(|doc| find_all(&doc, query))
                .unwrap_or_default()
                .into_iter()
                .map(move |m| (i, m))
        })
    }

    /// The images of one chapter, resolved relative to that chapter's
    /// location in the book. Pass this to layout and painting.
    pub fn chapter_images(&self, index: usize) -> ChapterImages<'_> {
        ChapterImages {
            book: self,
            dir: self
                .spine
                .get(index)
                .map(|p| parent_dir(p))
                .unwrap_or_default(),
        }
    }

    /// Raw bytes of any file in the book (images, CSS, fonts).
    pub fn resource(&self, path: &str) -> Result<Vec<u8>> {
        self.zip.read(path)
    }
}

/// Image access for one chapter; see [`EpubBook::chapter_images`].
pub struct ChapterImages<'a> {
    book: &'a EpubBook,
    dir: String,
}

impl ImageProvider for ChapterImages<'_> {
    fn bytes(&self, src: &str) -> Option<Vec<u8>> {
        if src.starts_with("data:") || src.contains("://") {
            return None;
        }
        self.book
            .zip
            .read(&join_path(&self.dir, &percent_decode(src)))
            .ok()
    }
}

/// Font obfuscation (IDPF and Adobe) "encrypts" embedded fonts only; the
/// text is readable. Anything else in `encryption.xml`, or an Adobe
/// `rights.xml`, means real DRM.
fn is_drm_protected(zip: &ZipArchive) -> bool {
    const FONT_OBFUSCATION: [&str; 2] = [
        "http://www.idpf.org/2008/embedding",
        "http://ns.adobe.com/pdf/enc#RC",
    ];
    if zip.contains("META-INF/rights.xml") {
        return true;
    }
    let Ok(xml) = zip.read_string("META-INF/encryption.xml") else {
        return false;
    };
    let mut algorithm: Option<String> = None;
    for tok in Tokenizer::new(&xml) {
        match tok {
            Token::Start { name, .. } if name == "encrypteddata" => algorithm = None,
            Token::Start { name, attrs, .. } if name == "encryptionmethod" => {
                algorithm = attr(&attrs, "algorithm")
            }
            Token::Start { name, attrs, .. } if name == "cipherreference" => {
                let uri = attr(&attrs, "uri").unwrap_or_default().to_ascii_lowercase();
                let is_font = [".ttf", ".otf", ".woff", ".woff2"]
                    .iter()
                    .any(|e| uri.ends_with(e));
                let obfuscation = algorithm
                    .as_deref()
                    .is_some_and(|a| FONT_OBFUSCATION.contains(&a));
                if !(obfuscation && is_font) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn attr(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..=i].to_string(),
        None => String::new(),
    }
}

/// Joins a relative href onto a directory and normalises `..` and `.`.
fn join_path(dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let full = if href.starts_with('/') {
        href.trim_start_matches('/').to_string()
    } else {
        format!("{dir}{href}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in full.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_nav(html: &str, dir: &str) -> Vec<TocEntry> {
    let mut out = Vec::new();
    let mut in_toc = false;
    let mut nav_depth = 0u32;
    let mut list_depth = 0u8;
    let mut href: Option<String> = None;
    let mut title = String::new();
    for tok in Tokenizer::new(html) {
        match tok {
            Token::Start { name, attrs, .. } => match name.as_str() {
                "nav" => {
                    nav_depth += 1;
                    if attr(&attrs, "type").as_deref() == Some("toc") {
                        in_toc = true;
                    }
                }
                "ol" if in_toc => list_depth += 1,
                "a" if in_toc => {
                    href = attr(&attrs, "href");
                    title.clear();
                }
                _ => {}
            },
            Token::Text(t) if href.is_some() => title.push_str(&t),
            Token::End { name } => match name.as_str() {
                "a" => {
                    if let Some(h) = href.take() {
                        out.push(TocEntry {
                            title: title.split_whitespace().collect::<Vec<_>>().join(" "),
                            href: join_path(dir, &percent_decode(&h)),
                            depth: list_depth.saturating_sub(1),
                        });
                    }
                }
                "ol" if in_toc => list_depth = list_depth.saturating_sub(1),
                "nav" => {
                    nav_depth = nav_depth.saturating_sub(1);
                    if nav_depth == 0 {
                        in_toc = false;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

fn parse_ncx(xml: &str, dir: &str) -> Vec<TocEntry> {
    let mut out = Vec::new();
    let mut depth = 0u8;
    let mut in_label = false;
    let mut title = String::new();
    for tok in Tokenizer::new(xml) {
        match tok {
            Token::Start { name, attrs, .. } => match name.as_str() {
                "navpoint" => {
                    depth += 1;
                    title.clear();
                }
                "text" => in_label = true,
                "content" => {
                    if let Some(src) = attr(&attrs, "src") {
                        out.push(TocEntry {
                            title: title.trim().to_string(),
                            href: join_path(dir, &percent_decode(&src)),
                            depth: depth.saturating_sub(1),
                        });
                    }
                }
                _ => {}
            },
            Token::Text(t) if in_label => title.push_str(&t),
            Token::End { name } => match name.as_str() {
                "text" => in_label = false,
                "navpoint" => depth = depth.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            join_path("OEBPS/", "Text/ch1.xhtml#x"),
            "OEBPS/Text/ch1.xhtml"
        );
        assert_eq!(
            join_path("OEBPS/Text/", "../Images/a.png"),
            "OEBPS/Images/a.png"
        );
        assert_eq!(percent_decode("a%20b.xhtml"), "a b.xhtml");
    }

    fn zip_with(files: &[(&str, &str)]) -> ZipArchive {
        // A stored (uncompressed) zip, written by hand.
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
            out.extend_from_slice(body.as_bytes());
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
        ZipArchive::open(Source::from_vec(out)).unwrap()
    }

    #[test]
    fn font_obfuscation_is_not_drm() {
        let enc = |alg: &str, uri: &str| {
            format!(
                r#"<encryption><EncryptedData><EncryptionMethod Algorithm="{alg}"/>
                <CipherData><CipherReference URI="{uri}"/></CipherData></EncryptedData></encryption>"#
            )
        };
        let idpf = enc("http://www.idpf.org/2008/embedding", "OEBPS/Misc/Font.otf");
        let z = zip_with(&[("META-INF/encryption.xml", &idpf)]);
        assert!(z.read_string("META-INF/encryption.xml").is_ok());
        assert!(!is_drm_protected(&z));
        let adept = enc(
            "http://www.w3.org/2001/04/xmlenc#aes128-cbc",
            "OEBPS/Text/ch1.xhtml",
        );
        assert!(is_drm_protected(&zip_with(&[(
            "META-INF/encryption.xml",
            &adept
        )])));
        assert!(is_drm_protected(&zip_with(&[(
            "META-INF/rights.xml",
            "<rights/>"
        )])));
        assert!(!is_drm_protected(&zip_with(&[(
            "mimetype",
            "application/epub+zip"
        )])));
    }

    #[test]
    fn opens_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/indic-sample.epub"
        );
        let book = EpubBook::open(Source::open(path).unwrap()).unwrap();
        assert_eq!(book.metadata.language.as_deref(), Some("bn"));
        assert!(book.chapter_count() >= 3);
        assert!(!book.toc.is_empty());
        let ch = book.chapter(0).unwrap();
        assert!(ch.blocks.iter().any(|b| b.text.contains("ক্ষ")));
        // The linked stylesheet centres headings and indents paragraphs.
        assert_eq!(ch.blocks[0].style.align, Some(akkhor_flow::Align::Center));
        assert_eq!(ch.blocks[1].style.indent, Some(1.5));
        // ক্ষ is in the Bangla chapter (several times) and in the English one.
        let hits: Vec<_> = book.search("ক্ষ").collect();
        assert!(hits.iter().any(|(c, _)| *c == 0));
        assert!(hits.iter().any(|(c, _)| *c == 2));
        assert!(book.search("उज्ज्वल").any(|(c, _)| c == 1));
        // The English chapter has a picture, resolved relative to the chapter.
        let ch3 = book.chapter(2).unwrap();
        let img = ch3.blocks.iter().find(|b| b.image.is_some()).unwrap();
        assert_eq!(
            book.chapter_images(2).size(img.image.as_deref().unwrap()),
            Some((600, 300))
        );
    }
}
