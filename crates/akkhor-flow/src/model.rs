//! The one document model every reflowable format is converted into.
//!
//! A document is a flat list of blocks (paragraphs, headings, images, ...).
//! Each block holds its text as one string plus style spans over byte
//! ranges, and the block-level style the book's CSS gave it. Reading
//! positions are `(block, byte offset)`, so they survive any change of font,
//! size or screen.

use crate::css::{Align, Decl, ElementInfo, Stylesheet, parse_declarations};
use crate::markup::{Token, Tokenizer};

/// A place in a document: a block and a byte offset into its text.
/// Positions order the way the text reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub block: usize,
    pub offset: usize,
}

impl Position {
    pub fn new(block: usize, offset: usize) -> Self {
        Self { block, offset }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleSpan {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    /// Heading level 1 to 6.
    Heading(u8),
    Quote,
    ListItem,
    Preformatted,
    /// An image; `Block::image` holds its source and `text` its alt text.
    Image,
}

/// Block-level presentation from the book's CSS. `None` means "not set":
/// the layout uses its own default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockStyle {
    pub align: Option<Align>,
    /// First-line indent, in ems.
    pub indent: Option<f32>,
    /// Space above and below, in ems.
    pub margin_top: Option<f32>,
    pub margin_bottom: Option<f32>,
    /// Font size relative to the body text.
    pub font_scale: Option<f32>,
    pub page_break_before: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    /// Text with whitespace collapsed. A `'\n'` is a forced line break.
    pub text: String,
    pub spans: Vec<StyleSpan>,
    pub style: BlockStyle,
    /// Source path or URL of an image block, as written in the book.
    pub image: Option<String>,
}

impl Block {
    pub fn paragraph(text: impl Into<String>) -> Self {
        Block {
            kind: BlockKind::Paragraph,
            text: text.into(),
            spans: Vec::new(),
            style: BlockStyle::default(),
            image: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// BCP 47 language from the source, if it declared one.
    pub lang: Option<String>,
}

impl Document {
    /// Builds a document from HTML or XHTML (EPUB chapters, MOBI text).
    /// `<style>` blocks and `style` attributes are honoured.
    pub fn from_html(html: &str) -> Self {
        Self::from_html_with_css(html, &[])
    }

    /// Like [`from_html`](Self::from_html), with linked stylesheets the
    /// caller has already loaded (EPUB `<link rel="stylesheet">`).
    pub fn from_html_with_css(html: &str, stylesheets: &[&str]) -> Self {
        let mut b = Builder::default();
        for css in stylesheets {
            b.sheet.add(css);
        }
        for tok in Tokenizer::new(html) {
            b.token(tok);
        }
        b.finish()
    }

    /// Builds a document from plain text. Files that separate paragraphs
    /// with blank lines (and may hard-wrap lines inside them) are read that
    /// way; files with one paragraph per line, common for Bangla text, keep
    /// every line as its own paragraph.
    pub fn from_plain_text(text: &str) -> Self {
        let mut doc = Document::default();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
        let blank_separated = text.matches("\n\n").count();
        let one_per_line = lines >= 8 && blank_separated * 8 < lines;
        let paragraphs: Box<dyn Iterator<Item = &str>> = if one_per_line {
            Box::new(text.lines())
        } else {
            Box::new(text.split("\n\n"))
        };
        for para in paragraphs.map(str::trim).filter(|p| !p.is_empty()) {
            doc.blocks.push(Block::paragraph(collapse_ws(para)));
        }
        doc
    }

    /// Total characters, a rough measure of length for progress bars.
    pub fn char_count(&self) -> usize {
        self.blocks.iter().map(|b| b.text.chars().count()).sum()
    }
}

/// `href`s of `<link rel="stylesheet">` elements, in document order.
pub fn linked_stylesheets(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in Tokenizer::new(html) {
        match tok {
            Token::Start { name, attrs, .. } if name == "link" => {
                let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
                if get("rel").is_some_and(|r| r.to_ascii_lowercase().contains("stylesheet"))
                    && let Some(href) = get("href")
                {
                    out.push(href.to_string());
                }
            }
            Token::Start { name, .. } if name == "body" => break,
            _ => {}
        }
    }
    out
}

/// Inherited computed style of an open element.
#[derive(Clone, Debug, Default)]
struct Computed {
    bold: bool,
    italic: bool,
    hidden: bool,
    pre: bool,
    align: Option<Align>,
    indent: Option<f32>,
    font_scale: Option<f32>,
}

struct Frame {
    computed: Computed,
    opens_block: bool,
}

/// Deeper nesting than this is treated as flat: real books never get close,
/// and it bounds the work hostile files can cause.
const MAX_DEPTH: usize = 256;

#[derive(Default)]
struct Builder {
    doc: Document,
    sheet: Stylesheet,
    stack: Vec<Frame>,
    /// Selector facts of the open elements, parallel to `stack`.
    infos: Vec<ElementInfo>,
    kind: Option<BlockKind>,
    block_style: Option<BlockStyle>,
    text: String,
    spans: Vec<StyleSpan>,
    span_start: usize,
    /// Text of the `<style>` element being read.
    css: Option<String>,
}

const VOID: &[&str] = &[
    "br", "img", "image", "hr", "meta", "link", "input", "col", "area", "base", "wbr", "source",
];

impl Builder {
    fn top(&self) -> Computed {
        self.stack
            .last()
            .map(|f| f.computed.clone())
            .unwrap_or_default()
    }

    fn style(&self) -> Style {
        let c = self.top();
        Style {
            bold: c.bold,
            italic: c.italic,
        }
    }

    fn token(&mut self, tok: Token<'_>) {
        match tok {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => self.open(name, attrs, self_closing),
            Token::End { name } => self.close(&name),
            Token::Text(t) => {
                if let Some(css) = self.css.as_mut() {
                    css.push_str(&t);
                } else if !self.top().hidden {
                    self.push_text(&t);
                }
            }
        }
    }

    fn open(&mut self, name: String, attrs: Vec<(String, String)>, self_closing: bool) {
        let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        if name == "html"
            && let Some(lang) = get("lang")
        {
            self.doc.lang = Some(lang);
        }
        let info = ElementInfo {
            id: get("id"),
            classes: get("class")
                .map(|c| c.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            name,
        };
        let mut decls = self.sheet.matching(&info, &self.infos);
        if let Some(inline) = get("style") {
            decls.extend(parse_declarations(&inline));
        }

        // Computed style: inherit, apply the element's defaults, then CSS.
        let parent = self.top();
        let mut c = parent.clone();
        let name = info.name.as_str();
        match name {
            "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th" => c.bold = true,
            "i" | "em" | "cite" | "dfn" | "var" => c.italic = true,
            "head" | "script" | "style" | "title" | "template" => c.hidden = true,
            "pre" => c.pre = true,
            _ => {}
        }
        let mut block = BlockStyle::default();
        for d in &decls {
            match *d {
                Decl::Bold(v) => c.bold = v,
                Decl::Italic(v) => c.italic = v,
                Decl::DisplayNone(v) => c.hidden |= v,
                Decl::TextAlign(a) => c.align = Some(a),
                Decl::TextIndent(v) => c.indent = Some(v),
                Decl::FontSize(f) => c.font_scale = Some(parent.font_scale.unwrap_or(1.0) * f),
                Decl::MarginTop(v) => block.margin_top = Some(v),
                Decl::MarginBottom(v) => block.margin_bottom = Some(v),
                Decl::PageBreakBefore(v) => block.page_break_before = v,
            }
        }

        // Images become blocks of their own.
        if (name == "img" || name == "image") && !c.hidden {
            let src = get("src").or_else(|| get("href"));
            if let Some(src) = src {
                self.flush();
                let alt = get("alt").unwrap_or_default();
                self.doc.blocks.push(Block {
                    kind: BlockKind::Image,
                    text: alt,
                    spans: Vec::new(),
                    style: block,
                    image: Some(src),
                });
            }
            return;
        }
        if name == "br" {
            if !c.hidden {
                self.push_break();
            }
            return;
        }

        let kind = block_kind(name);
        let opens_block = kind.is_some() || is_block(name) || name == "pre";
        if opens_block && !c.hidden {
            self.flush();
            self.kind = kind.or(if name == "pre" {
                Some(BlockKind::Preformatted)
            } else {
                None
            });
            block.align = c.align;
            block.indent = c.indent;
            block.font_scale = c.font_scale;
            // A page break carried from an empty element before this one.
            let carried = self.block_style.take().is_some_and(|b| b.page_break_before);
            block.page_break_before |= carried;
            self.block_style = Some(block);
        }
        if self_closing || VOID.contains(&name) || self.stack.len() >= MAX_DEPTH {
            return;
        }
        if name == "style" {
            self.css = Some(String::new());
        }
        self.close_span();
        self.stack.push(Frame {
            computed: c,
            opens_block,
        });
        self.infos.push(info);
        self.span_start = self.text.len();
    }

    fn close(&mut self, name: &str) {
        let Some(at) = self.infos.iter().rposition(|i| i.name == name) else {
            return; // stray end tag
        };
        self.close_span();
        let mut block_closed = false;
        while self.stack.len() > at {
            let f = self.stack.pop().expect("non-empty");
            let info = self.infos.pop().expect("parallel to stack");
            block_closed |= f.opens_block && !f.computed.hidden;
            if info.name == "style"
                && let Some(css) = self.css.take()
            {
                self.sheet.add(&css);
            }
        }
        if block_closed {
            self.flush();
        }
        self.span_start = self.text.len();
    }

    fn close_span(&mut self) {
        let style = self.style();
        if style != Style::default() && self.text.len() > self.span_start {
            match self.spans.last_mut() {
                // Extend the previous span when the style simply continues.
                Some(last) if last.end == self.span_start && last.style == style => {
                    last.end = self.text.len()
                }
                _ => self.spans.push(StyleSpan {
                    start: self.span_start,
                    end: self.text.len(),
                    style,
                }),
            }
        }
        self.span_start = self.text.len();
    }

    fn push_text(&mut self, t: &str) {
        if self.top().pre {
            self.text.push_str(t);
            return;
        }
        for c in t.chars() {
            if c.is_ascii_whitespace() {
                if !self.text.is_empty() && !self.text.ends_with([' ', '\n']) {
                    self.text.push(' ');
                }
            } else {
                self.text.push(c);
            }
        }
    }

    /// An explicit `<br>`.
    fn push_break(&mut self) {
        trim_trailing_space(&mut self.text);
        self.text.push('\n');
    }

    fn flush(&mut self) {
        self.close_span();
        let kind = self.kind.take().unwrap_or(BlockKind::Paragraph);
        let top = self.top();
        let style = self.block_style.take().unwrap_or_else(|| BlockStyle {
            align: top.align,
            indent: top.indent,
            font_scale: top.font_scale,
            ..Default::default()
        });
        let text = std::mem::take(&mut self.text);
        let mut spans = std::mem::take(&mut self.spans);
        let trimmed_end = text.trim_end().len();
        let lead = text.len() - text.trim_start().len();
        if trimmed_end > lead {
            let text = text[lead..trimmed_end].to_string();
            spans.retain_mut(|s| {
                s.start = s.start.saturating_sub(lead).min(text.len());
                s.end = s.end.saturating_sub(lead).min(text.len());
                s.end > s.start
            });
            self.doc.blocks.push(Block {
                kind,
                text,
                spans,
                style,
                image: None,
            });
        } else if style.page_break_before {
            // An empty element that only asks for a page break: carry it to
            // the next block.
            self.block_style = Some(BlockStyle {
                page_break_before: true,
                ..Default::default()
            });
        }
        self.span_start = 0;
    }

    fn finish(mut self) -> Document {
        self.flush();
        self.doc
    }
}

fn trim_trailing_space(s: &mut String) {
    while s.ends_with(' ') {
        s.pop();
    }
}

fn block_kind(name: &str) -> Option<BlockKind> {
    Some(match name {
        "p" => BlockKind::Paragraph,
        "h1" => BlockKind::Heading(1),
        "h2" => BlockKind::Heading(2),
        "h3" => BlockKind::Heading(3),
        "h4" => BlockKind::Heading(4),
        "h5" => BlockKind::Heading(5),
        "h6" => BlockKind::Heading(6),
        "blockquote" => BlockKind::Quote,
        "li" | "dd" | "dt" => BlockKind::ListItem,
        // FictionBook
        "v" => BlockKind::Paragraph,
        _ => return None,
    })
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "div"
            | "section"
            | "article"
            | "body"
            | "ul"
            | "ol"
            | "dl"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "hr"
            | "figure"
            | "figcaption"
            | "header"
            | "footer"
            | "nav"
            | "aside"
            | "main"
            | "svg"
            | "poem"
            | "stanza"
            | "epigraph"
    )
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_ascii_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_blocks_from_html() {
        let doc = Document::from_html(
            r#"<html lang="bn"><head><title>x</title><style>p{}</style></head>
            <body><h1> অধ্যায়   এক </h1>
            <p>আমার <b>সোনার</b> বাংলা,<br/> আমি তোমায় ভালোবাসি।</p>
            <div>नमस्ते <i>दुनिया</i></div></body></html>"#,
        );
        assert_eq!(doc.lang.as_deref(), Some("bn"));
        assert_eq!(doc.blocks.len(), 3);
        assert_eq!(doc.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(doc.blocks[0].text, "অধ্যায় এক");
        assert!(
            doc.blocks[0].spans[0].style.bold,
            "headings are bold by default"
        );
        let p = &doc.blocks[1];
        assert_eq!(p.text, "আমার সোনার বাংলা,\nআমি তোমায় ভালোবাসি।");
        assert_eq!(&p.text[p.spans[0].start..p.spans[0].end], "সোনার");
        assert!(p.spans[0].style.bold);
        let d = &doc.blocks[2];
        assert_eq!(&d.text[d.spans[0].start..d.spans[0].end], "दुनिया");
    }

    #[test]
    fn applies_css() {
        let doc = Document::from_html_with_css(
            r#"<html><head><style>
                .c { text-align: center; font-size: 1.2em }
                p.noindent { text-indent: 0 }
                .hide { display: none }
                h2 { font-weight: normal; page-break-before: always; margin-top: 3em }
                span.em { font-style: italic }
            </style></head><body>
            <p class="c">মাঝখানে</p>
            <p class="noindent">শুরু <span class="em">বাঁকা</span></p>
            <div class="hide"><p>লুকানো</p></div>
            <h2>দ্বিতীয়</h2>
            <p style="text-align:right; font-weight:bold">ডানে</p>
            <img src="../Images/cover.png" alt="প্রচ্ছদ"/>
            </body></html>"#,
            &["p { text-indent: 1.5em; text-align: justify }"],
        );
        let texts: Vec<&str> = doc.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, vec!["মাঝখানে", "শুরু বাঁকা", "দ্বিতীয়", "ডানে", "প্রচ্ছদ"]);
        let b = &doc.blocks;
        assert_eq!(b[0].style.align, Some(Align::Center));
        assert_eq!(b[0].style.font_scale, Some(1.2));
        assert_eq!(b[0].style.indent, Some(1.5), "linked sheet applies");
        assert_eq!(b[1].style.indent, Some(0.0));
        assert!(b[1].spans.iter().any(|s| s.style.italic));
        assert!(b[2].style.page_break_before);
        assert_eq!(b[2].style.margin_top, Some(3.0));
        assert!(
            b[2].spans.is_empty(),
            "font-weight: normal un-bolds the heading"
        );
        assert_eq!(b[3].style.align, Some(Align::End));
        assert!(b[3].spans[0].style.bold);
        assert_eq!(b[4].kind, BlockKind::Image);
        assert_eq!(b[4].image.as_deref(), Some("../Images/cover.png"));
    }

    #[test]
    fn finds_linked_stylesheets() {
        let html = r#"<html><head><link rel="stylesheet" href="../Styles/a.css"/>
            <link rel="icon" href="x.png"/><link href="b.css" rel="Stylesheet"></head><body/></html>"#;
        assert_eq!(linked_stylesheets(html), vec!["../Styles/a.css", "b.css"]);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    #[test]
    fn self_closing_style_does_not_swallow_text() {
        let doc = Document::from_html("<head><style/></head><body><p>দেখা যায়</p></body>");
        assert_eq!(doc.blocks.len(), 1, "{doc:?}");
    }

    #[test]
    fn plain_text_paragraph_styles() {
        let blank = "এক লাইন\nআরও এক\n\nদ্বিতীয় অনুচ্ছেদ\n";
        assert_eq!(Document::from_plain_text(blank).blocks.len(), 2);
        let per_line = (0..20).map(|i| format!("অনুচ্ছেদ {i}\n")).collect::<String>();
        assert_eq!(Document::from_plain_text(&per_line).blocks.len(), 20);
    }

    #[test]
    fn empty_page_break_element_carries_to_next_block() {
        let doc = Document::from_html(
            r#"<p>এক</p><div style="page-break-before:always"></div><p>দুই</p>"#,
        );
        assert!(doc.blocks[1].style.page_break_before, "{:?}", doc.blocks[1]);
    }
}
