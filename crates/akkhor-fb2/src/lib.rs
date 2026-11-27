//! FictionBook 2 (FB2) books for Akkhor, plain or zipped (`.fb2.zip`).
//!
//! An FB2 file is one XML document: a `description` (title, authors,
//! language, cover), one or more `body` elements (the text, and usually a
//! `notes` body for footnotes) and `binary` elements holding pictures as
//! base64. Each top-level section of the main body is a chapter; the
//! notes body is one more. FB2 elements are turned into the HTML the
//! layout understands, with a small style sheet of its own.

mod base64;

use std::collections::HashMap;
use std::sync::OnceLock;

use akkhor_core::zip::ZipArchive;
use akkhor_core::{Error, Result, Source};
use akkhor_flow::Document;
use akkhor_flow::encoding::{decode_cp1251, decode_cp1252, decode_koi8r};
use akkhor_flow::image::ImageProvider;
use akkhor_flow::markup::{Token, Tokenizer};

/// How FB2's elements look; the layout's own defaults do the rest.
const STYLE: &str = "h1,h2,h3,h4{text-align:center} \
    .subtitle{text-align:center;font-weight:bold} \
    .epigraph{margin-left:30%;font-style:italic} \
    .poem{margin-left:10%} .stanza{margin-top:1em} \
    .author{text-align:right;font-style:italic} \
    blockquote{margin-left:8%}";

/// FB2 files bigger than this are refused (the largest novels with
/// pictures are a few tens of megabytes).
const MAX_FILE: usize = 256 << 20;

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub annotation: Option<String>,
    pub series: Option<String>,
    /// `binary` id of the cover picture.
    pub cover: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TocEntry {
    pub title: String,
    pub chapter: usize,
    pub depth: u8,
}

pub struct Fb2Book {
    /// The whole document as text.
    xml: String,
    pub metadata: Metadata,
    chapters: OnceLock<Chapters>,
    binaries: OnceLock<HashMap<String, (usize, usize)>>,
}

struct Chapters {
    html: Vec<String>,
    toc: Vec<TocEntry>,
}

impl Fb2Book {
    pub fn open(source: Source) -> Result<Self> {
        let bytes = source.bytes();
        let raw: Vec<u8>;
        let data: &[u8] = if bytes.starts_with(b"PK\x03\x04") {
            let zip = ZipArchive::open(source.clone())?;
            let name = zip
                .entries()
                .iter()
                .find(|e| e.name.to_ascii_lowercase().ends_with(".fb2"))
                .map(|e| e.name.clone())
                .ok_or(Error::malformed("no .fb2 file in the zip"))?;
            raw = zip.read(&name)?;
            &raw
        } else {
            bytes
        };
        if data.len() > MAX_FILE {
            return Err(Error::malformed("FB2 file too large"));
        }
        let xml = decode(data);
        // Bytes, not text: 4096 may fall inside a letter.
        let head = &xml.as_bytes()[..xml.len().min(4096)];
        if !head.windows(12).any(|w| w == b"<FictionBook") {
            return Err(Error::malformed("not a FictionBook document"));
        }
        let metadata = metadata(&xml);
        Ok(Fb2Book {
            xml,
            metadata,
            chapters: OnceLock::new(),
            binaries: OnceLock::new(),
        })
    }

    fn chapters(&self) -> &Chapters {
        self.chapters.get_or_init(|| convert(&self.xml))
    }

    pub fn chapter_count(&self) -> usize {
        self.chapters().html.len()
    }

    pub fn toc(&self) -> Vec<TocEntry> {
        self.chapters().toc.clone()
    }

    pub fn chapter(&self, index: usize) -> Result<Document> {
        let html = self.chapters().html.get(index).ok_or(Error::OutOfRange)?;
        let mut doc = Document::from_html_with_css(html, &[STYLE]);
        if doc.lang.is_none() {
            doc.lang = self.metadata.language.clone();
        }
        Ok(doc)
    }

    pub fn chapter_sizes(&self) -> Vec<u64> {
        self.chapters()
            .html
            .iter()
            .map(|c| c.len().max(1) as u64)
            .collect()
    }

    pub fn chapter_images(&self, _index: usize) -> Images<'_> {
        Images { book: self }
    }

    pub fn cover_bytes(&self) -> Option<Vec<u8>> {
        self.binary(self.metadata.cover.as_deref()?)
    }

    /// A picture by its `binary` id, decoded from base64.
    fn binary(&self, id: &str) -> Option<Vec<u8>> {
        let map = self.binaries.get_or_init(|| binaries(&self.xml));
        let &(s, e) = map.get(id.trim_start_matches('#'))?;
        base64::decode(&self.xml.as_bytes()[s..e])
    }
}

/// Pictures of an FB2 book: `fb2:<id>` sources.
pub struct Images<'a> {
    book: &'a Fb2Book,
}

impl ImageProvider for Images<'_> {
    fn bytes(&self, src: &str) -> Option<Vec<u8>> {
        self.book.binary(src.strip_prefix("fb2:")?)
    }
}

/// The document's text in the encoding its XML declaration names.
fn decode(b: &[u8]) -> String {
    if b.starts_with(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(&b[3..]).into_owned();
    }
    if b.starts_with(b"\xFF\xFE") || b.starts_with(b"\xFE\xFF") {
        return akkhor_flow::encoding::decode_text(b).text;
    }
    let head = String::from_utf8_lossy(&b[..b.len().min(200)]).to_ascii_lowercase();
    let enc = head
        .split("encoding=")
        .nth(1)
        .and_then(|r| r.trim_start_matches(['"', '\'']).split(['"', '\'']).next())
        .unwrap_or("utf-8")
        .to_string();
    match enc.as_str() {
        "windows-1251" | "cp1251" => decode_cp1251(b),
        "koi8-r" => decode_koi8r(b),
        "windows-1252" | "iso-8859-1" | "latin1" | "cp1252" => decode_cp1252(b),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key || k.rsplit(':').next() == Some(key))
        .map(|(_, v)| v.as_str())
}

/// The `description`: title, authors, language, publisher, cover.
fn metadata(xml: &str) -> Metadata {
    let end = xml
        .find("</description>")
        .map_or(xml.len().min(1 << 20), |i| i);
    let mut m = Metadata::default();
    let mut path: Vec<String> = Vec::new();
    let mut author: Vec<String> = Vec::new();
    let mut text = String::new();
    for tok in Tokenizer::new(&xml[..end]) {
        match tok {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => {
                if name == "image" && path.iter().any(|p| p == "coverpage") && m.cover.is_none() {
                    m.cover = attr(&attrs, "href").map(|h| h.trim_start_matches('#').to_string());
                }
                if name == "sequence" && path.last().is_some_and(|p| p == "title-info") {
                    m.series = attr(&attrs, "name").map(str::to_string);
                }
                if !self_closing {
                    path.push(name);
                    text.clear();
                }
            }
            Token::Text(t) => text.push_str(&t),
            Token::End { name } => {
                let in_title = path.iter().any(|p| p == "title-info");
                let in_publish = path.iter().any(|p| p == "publish-info");
                let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
                match name.as_str() {
                    "book-title" if in_title && !t.is_empty() => m.title = Some(t.clone()),
                    "lang" if in_title && !t.is_empty() => m.language = Some(t.clone()),
                    "publisher" if in_publish && !t.is_empty() => m.publisher = Some(t.clone()),
                    "first-name" | "middle-name" | "last-name" | "nickname"
                        if in_title && path.iter().any(|p| p == "author") && !t.is_empty() =>
                    {
                        author.push(t.clone());
                    }
                    "author" if in_title => {
                        if !author.is_empty() {
                            m.authors.push(author.join(" "));
                        }
                        author.clear();
                    }
                    "annotation" if in_title => {}
                    _ => {}
                }
                if name == "p" && in_title && path.iter().any(|p| p == "annotation") {
                    let a = m.annotation.get_or_insert_with(String::new);
                    if !a.is_empty() {
                        a.push('\n');
                    }
                    a.push_str(&t);
                }
                if let Some(i) = path.iter().rposition(|p| *p == name) {
                    path.truncate(i);
                }
                text.clear();
            }
        }
    }
    m
}

/// Byte ranges of every `binary` element's base64 text, by id.
fn binaries(xml: &str) -> HashMap<String, (usize, usize)> {
    let mut map = HashMap::new();
    let mut at = 0;
    while let Some(i) = xml[at..].find("<binary") {
        let start = at + i;
        let Some(gt) = xml[start..].find('>') else {
            break;
        };
        let tag = &xml[start..start + gt];
        let body = start + gt + 1;
        let Some(end) = xml[body..].find("</binary>") else {
            break;
        };
        if let Some(id) = tag
            .split("id=")
            .nth(1)
            .and_then(|r| r.trim_start_matches(['"', '\'']).split(['"', '\'']).next())
        {
            map.insert(id.to_string(), (body, body + end));
        }
        at = body + end;
    }
    map
}

/// The bodies as chapters of HTML: each top-level section of the main
/// body, and the notes body.
fn convert(xml: &str) -> Chapters {
    let mut html: Vec<String> = Vec::new();
    let mut toc = Vec::new();
    let mut cur = String::new();
    let mut in_body = false;
    let mut notes = false;
    let mut depth = 0usize; // section depth inside the body
    let mut skip = 0usize; // inside elements whose text is not shown
    // Title of the chapter being built, and whether its first paragraph
    // was all bold (a heading in files without `title`).
    let mut title = String::new();
    let mut in_title = 0usize;
    let mut first_p: Option<(String, bool)> = None;
    let mut bold = 0usize;
    let mut p_text = String::new();
    let mut p_plain = false;
    let mut in_p = false;
    // Open FB2 elements, to know what a `title` belongs to.
    let mut open: Vec<String> = Vec::new();
    // Whether the title being read is the chapter's own.
    let mut chapter_title = false;
    // Heading level of each open title, and the text of the current one.
    let mut levels: Vec<usize> = Vec::new();
    let mut title_text = String::new();
    let flush = |cur: &mut String, html: &mut Vec<String>| {
        if cur.contains(|c: char| !c.is_whitespace()) {
            html.push(format!("<html><body>{cur}</body></html>"));
        }
        cur.clear();
    };
    for tok in Tokenizer::new(xml) {
        match tok {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => {
                if name == "body" && !self_closing {
                    in_body = true;
                    notes = attr(&attrs, "name").is_some_and(|n| n == "notes" || n == "comments");
                    flush(&mut cur, &mut html);
                    if notes {
                        toc.push(TocEntry {
                            title: "Notes".into(),
                            chapter: html.len(),
                            depth: 0,
                        });
                    }
                    continue;
                }
                if !in_body || skip > 0 {
                    if name == "binary" && !self_closing {
                        skip += 1;
                    }
                    continue;
                }
                let parent = open.last().cloned().unwrap_or_default();
                if !self_closing {
                    open.push(name.clone());
                }
                match name.as_str() {
                    "section" if !self_closing => {
                        depth += 1;
                        if depth == 1 && !notes {
                            flush(&mut cur, &mut html);
                            title.clear();
                            first_p = None;
                        }
                        if let Some(id) = attr(&attrs, "id") {
                            cur.push_str(&format!("<div id=\"{}\">", escape(id)));
                        } else {
                            cur.push_str("<div>");
                        }
                    }
                    "title" if !self_closing => {
                        in_title += 1;
                        chapter_title = parent == "section" && depth == 1 && !notes;
                        title_text.clear();
                        // Section titles by depth; a poem's or the body's
                        // own title smaller or larger.
                        let h = match parent.as_str() {
                            "section" => depth.clamp(1, 4),
                            "body" => 1,
                            _ => 4,
                        };
                        levels.push(h);
                        cur.push_str(&format!("<h{h}>"));
                    }
                    "p" if !self_closing => {
                        in_p = true;
                        p_text.clear();
                        p_plain = false;
                        // A title's lines are lines of one heading.
                        cur.push_str(if in_title > 0 { "<span>" } else { "<p>" });
                    }
                    "subtitle" if !self_closing => cur.push_str("<p class=\"subtitle\">"),
                    "epigraph" if !self_closing => cur.push_str("<div class=\"epigraph\">"),
                    "cite" if !self_closing => cur.push_str("<blockquote>"),
                    "poem" if !self_closing => cur.push_str("<div class=\"poem\">"),
                    "stanza" if !self_closing => cur.push_str("<div class=\"stanza\">"),
                    "v" if !self_closing => cur.push_str("<p class=\"v\">"),
                    "text-author" if !self_closing => cur.push_str("<p class=\"author\">"),
                    "empty-line" => cur.push_str("<p>\u{a0}</p>"),
                    "emphasis" if !self_closing => cur.push_str("<em>"),
                    "strong" if !self_closing => {
                        bold += 1;
                        cur.push_str("<strong>");
                    }
                    "strikethrough" if !self_closing => cur.push_str("<s>"),
                    "sub" | "sup" | "code" | "table" | "tr" | "td" | "th" if !self_closing => {
                        cur.push_str(&format!("<{name}>"));
                    }
                    "a" if !self_closing => {
                        let href = attr(&attrs, "href").unwrap_or("");
                        cur.push_str(&format!("<a href=\"{}\">", escape(href)));
                    }
                    "image" => {
                        if let Some(h) = attr(&attrs, "href") {
                            let alt = attr(&attrs, "alt").unwrap_or("");
                            let img = format!(
                                "<img src=\"fb2:{}\" alt=\"{}\"/>",
                                escape(h.trim_start_matches('#')),
                                escape(alt)
                            );
                            if in_p {
                                cur.push_str(&img);
                            } else {
                                cur.push_str(&format!("<p>{img}</p>"));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Token::End { name } => {
                if name == "binary" && skip > 0 {
                    skip -= 1;
                    continue;
                }
                if !in_body || skip > 0 {
                    continue;
                }
                if let Some(i) = open.iter().rposition(|o| *o == name) {
                    open.truncate(i);
                }
                match name.as_str() {
                    "body" => {
                        flush(&mut cur, &mut html);
                        in_body = false;
                        depth = 0;
                    }
                    "section" => {
                        cur.push_str("</div>");
                        if depth == 1 && !notes {
                            // A section without a title, whose first
                            // paragraph is all bold: that paragraph is its
                            // heading (as converters write them).
                            if title.is_empty()
                                && let Some((t, true)) = first_p.take()
                                && t.chars().count() <= 120
                            {
                                title = t;
                                // Only a whole paragraph is rewritten; a
                                // broken file may never close it.
                                if let Some(i) = cur.find("<p><strong>")
                                    && let Some(j) = cur[i..].find("</p>")
                                {
                                    let close = i + j;
                                    let inner = cur[i + 3..close].to_string();
                                    cur.replace_range(i..close + 4, &format!("<h1>{inner}</h1>"));
                                }
                            }
                            if !title.is_empty() {
                                toc.push(TocEntry {
                                    title: title.clone(),
                                    chapter: html.len(),
                                    depth: 0,
                                });
                            }
                        }
                        depth = depth.saturating_sub(1);
                    }
                    "title" => {
                        in_title = in_title.saturating_sub(1);
                        chapter_title = false;
                        let h = levels.pop().unwrap_or(1);
                        if notes
                            && depth == 0
                            && !title_text.trim().is_empty()
                            && let Some(last) = toc.last_mut()
                        {
                            last.title =
                                title_text.split_whitespace().collect::<Vec<_>>().join(" ");
                        }
                        if cur.ends_with("<br/>") {
                            cur.truncate(cur.len() - 5);
                        }
                        cur.push_str(&format!("</h{h}>"));
                    }
                    "p" => {
                        in_p = false;
                        if in_title > 0 {
                            cur.push_str("</span><br/>");
                            if chapter_title {
                                if !title.is_empty() {
                                    title.push(' ');
                                }
                                title.push_str(p_text.trim());
                            }
                        } else {
                            cur.push_str("</p>");
                            if first_p.is_none() && depth == 1 && !notes {
                                let t = p_text.split_whitespace().collect::<Vec<_>>().join(" ");
                                if !t.is_empty() {
                                    first_p = Some((t, !p_plain));
                                }
                            }
                        }
                    }
                    "subtitle" | "v" | "text-author" => cur.push_str("</p>"),
                    "epigraph" | "poem" | "stanza" => cur.push_str("</div>"),
                    "cite" => cur.push_str("</blockquote>"),
                    "emphasis" => cur.push_str("</em>"),
                    "strong" => {
                        bold = bold.saturating_sub(1);
                        cur.push_str("</strong>");
                    }
                    "strikethrough" => cur.push_str("</s>"),
                    "sub" | "sup" | "code" | "table" | "tr" | "td" | "th" | "a" => {
                        cur.push_str(&format!("</{name}>"));
                    }
                    _ => {}
                }
            }
            Token::Text(t) => {
                if !in_body || skip > 0 {
                    continue;
                }
                if in_title > 0 {
                    title_text.push_str(&t);
                    title_text.push(' ');
                }
                if in_p {
                    p_text.push_str(&t);
                    if bold == 0 && t.contains(|c: char| !c.is_whitespace()) {
                        p_plain = true;
                    }
                }
                cur.push_str(&escape(&t));
            }
        }
    }
    flush(&mut cur, &mut html);
    Chapters { html, toc }
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}
