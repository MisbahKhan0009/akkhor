//! The one document model every reflowable format is converted into.
//!
//! A document is a flat list of blocks (paragraphs, headings, ...). Each
//! block holds its text as one string plus style spans over byte ranges.
//! Reading positions are `(block, byte offset)`, so they survive any change
//! of font, size or screen.

use crate::markup::{Token, Tokenizer};

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Text with whitespace collapsed. A `'\n'` is a forced line break.
    pub text: String,
    pub spans: Vec<StyleSpan>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// BCP 47 language from the source, if it declared one.
    pub lang: Option<String>,
}

impl Document {
    /// Builds a document from HTML or XHTML (EPUB chapters, MOBI text).
    pub fn from_html(html: &str) -> Self {
        let mut b = Builder::default();
        for tok in Tokenizer::new(html) {
            b.token(tok);
        }
        b.finish()
    }

    /// Builds a document from plain text: blank lines separate paragraphs.
    pub fn from_plain_text(text: &str) -> Self {
        let mut doc = Document::default();
        for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
            doc.blocks.push(Block {
                kind: BlockKind::Paragraph,
                text: collapse_ws(para),
                spans: Vec::new(),
            });
        }
        doc
    }

    /// Total characters, a rough measure of length for progress bars.
    pub fn char_count(&self) -> usize {
        self.blocks.iter().map(|b| b.text.chars().count()).sum()
    }
}

#[derive(Default)]
struct Builder {
    doc: Document,
    kind: Option<BlockKind>,
    text: String,
    spans: Vec<StyleSpan>,
    bold: u32,
    italic: u32,
    pre: u32,
    /// Inside <head>, <script>, <style> or <title>: text is not content.
    hidden: u32,
    span_start: usize,
}

impl Builder {
    fn style(&self) -> Style {
        Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
        }
    }

    fn token(&mut self, tok: Token<'_>) {
        match tok {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => {
                if name == "html"
                    && let Some((_, lang)) = attrs.iter().find(|(k, _)| k == "lang")
                {
                    self.doc.lang = Some(lang.clone());
                }
                if self_closing {
                    if name == "br" {
                        self.push_break();
                    } else if is_block(&name) {
                        self.flush();
                    }
                    return;
                }
                self.open(&name);
            }
            Token::End { name } => self.close(&name),
            Token::Text(t) => {
                if self.hidden == 0 {
                    self.push_text(&t);
                }
            }
        }
    }

    fn open(&mut self, name: &str) {
        match name {
            "head" | "script" | "style" | "title" => self.hidden += 1,
            "b" | "strong" => self.style_change(|s| s.bold += 1),
            "i" | "em" | "cite" => self.style_change(|s| s.italic += 1),
            "br" => self.push_break(),
            "pre" => {
                self.flush();
                self.pre += 1;
                self.kind = Some(BlockKind::Preformatted);
            }
            _ => {
                if let Some(kind) = block_kind(name) {
                    self.flush();
                    self.kind = Some(kind);
                } else if is_block(name) {
                    self.flush();
                }
            }
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "head" | "script" | "style" | "title" => self.hidden = self.hidden.saturating_sub(1),
            "b" | "strong" => self.style_change(|s| s.bold = s.bold.saturating_sub(1)),
            "i" | "em" | "cite" => self.style_change(|s| s.italic = s.italic.saturating_sub(1)),
            "pre" => {
                self.flush();
                self.pre = self.pre.saturating_sub(1);
            }
            _ if is_block(name) || block_kind(name).is_some() => self.flush(),
            _ => {}
        }
    }

    fn style_change(&mut self, f: impl FnOnce(&mut Self)) {
        self.close_span();
        f(self);
        self.span_start = self.text.len();
    }

    fn close_span(&mut self) {
        let style = self.style();
        if style != Style::default() && self.text.len() > self.span_start {
            self.spans.push(StyleSpan {
                start: self.span_start,
                end: self.text.len(),
                style,
            });
        }
        self.span_start = self.text.len();
    }

    fn push_text(&mut self, t: &str) {
        if self.pre > 0 {
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
            self.doc.blocks.push(Block { kind, text, spans });
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
            | "img"
            | "image"
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
        let p = &doc.blocks[1];
        assert_eq!(p.text, "আমার সোনার বাংলা,\nআমি তোমায় ভালোবাসি।");
        assert_eq!(&p.text[p.spans[0].start..p.spans[0].end], "সোনার");
        assert!(p.spans[0].style.bold);
        let d = &doc.blocks[2];
        assert_eq!(&d.text[d.spans[0].start..d.spans[0].end], "दुनिया");
    }
}
