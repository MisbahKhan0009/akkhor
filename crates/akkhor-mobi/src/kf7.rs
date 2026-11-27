//! MOBI 7 markup as HTML the layout understands: presentational
//! attributes (`width` as first-line indent, `height` as space above,
//! `align`, `<font size>`) become CSS, and a paragraph that is all large
//! type becomes a heading, as MOBI 7 has no other way to mark one.

use akkhor_flow::markup::{Token, Tokenizer};

/// Font sizes 1 to 7 in ems, as Kindle readers scale them.
const SIZES: [f32; 8] = [1.0, 0.63, 0.82, 1.0, 1.13, 1.5, 2.0, 3.0];

pub(crate) fn normalize(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + html.len() / 8);
    // A paragraph is held back until its end, to see whether it is a heading.
    let mut para: Option<Para> = None;
    let mut sizes: Vec<u8> = Vec::new();
    for tok in Tokenizer::new(html) {
        match tok {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => {
                if name == "p" && !self_closing {
                    if let Some(p) = para.take() {
                        out.push_str(&p.finish());
                    }
                    para = Some(Para {
                        attrs: style_attrs(&name, &attrs),
                        body: String::new(),
                        text: 0,
                        big_text: 0,
                        biggest: 0,
                    });
                    continue;
                }
                let mut tag = String::new();
                if name == "font" {
                    let size = attr(&attrs, "size").and_then(font_size).unwrap_or(3);
                    if !self_closing {
                        sizes.push(size);
                    }
                    tag.push_str("<span");
                    if size != 3 {
                        tag.push_str(&format!(" style=\"font-size:{}em\"", SIZES[size as usize]));
                    }
                } else {
                    tag.push('<');
                    tag.push_str(&name);
                    tag.push_str(&style_attrs(&name, &attrs));
                }
                tag.push_str(if self_closing { "/>" } else { ">" });
                match &mut para {
                    Some(p) => p.body.push_str(&tag),
                    None => out.push_str(&tag),
                }
            }
            Token::End { name } => {
                if name == "p" {
                    if let Some(p) = para.take() {
                        out.push_str(&p.finish());
                    }
                    continue;
                }
                let tag = if name == "font" {
                    sizes.pop();
                    "</span>".to_string()
                } else {
                    format!("</{name}>")
                };
                match &mut para {
                    Some(p) => p.body.push_str(&tag),
                    None => out.push_str(&tag),
                }
            }
            Token::Text(t) => {
                let escaped = escape(&t);
                match &mut para {
                    Some(p) => {
                        let n = t.trim().chars().count();
                        let size = sizes.last().copied().unwrap_or(3);
                        p.text += n;
                        if size >= 5 {
                            p.big_text += n;
                            p.biggest = p.biggest.max(size);
                        }
                        p.body.push_str(&escaped);
                    }
                    None => out.push_str(&escaped),
                }
            }
        }
    }
    if let Some(p) = para.take() {
        out.push_str(&p.finish());
    }
    out
}

struct Para {
    attrs: String,
    body: String,
    text: usize,
    big_text: usize,
    biggest: u8,
}

impl Para {
    fn finish(self) -> String {
        let heading = self.text > 0 && self.big_text == self.text && self.text <= 200;
        let tag = match (heading, self.biggest) {
            (true, 6..) => "h1",
            (true, _) => "h2",
            (false, _) => "p",
        };
        format!("<{tag}{}>{}</{tag}>", self.attrs, self.body)
    }
}

/// Attributes kept, with MOBI's presentational ones as a style.
fn style_attrs(name: &str, attrs: &[(String, String)]) -> String {
    let mut style = Vec::new();
    let mut out = String::new();
    for (k, v) in attrs {
        match k.as_str() {
            "width" if matches!(name, "p" | "div" | "blockquote") => {
                style.push(format!("text-indent:{}", length(v)));
            }
            "height" if matches!(name, "p" | "div" | "blockquote") => {
                style.push(format!("margin-top:{}", length(v)));
            }
            "align" => style.push(format!("text-align:{}", v.to_ascii_lowercase())),
            "style" => style.push(v.clone()),
            "filepos" | "recindex" | "valign" | "value" => {
                if k == "filepos" {
                    out.push_str(&format!(" href=\"#filepos{}\"", v.trim_start_matches('0')));
                }
            }
            _ => out.push_str(&format!(" {k}=\"{}\"", escape(v))),
        }
    }
    if !style.is_empty() {
        out.push_str(&format!(" style=\"{}\"", escape(&style.join(";"))));
    }
    out
}

fn length(v: &str) -> String {
    let v = v.trim();
    if v.chars().all(|c| c.is_ascii_digit() || c == '.') {
        format!("{v}px")
    } else {
        v.to_string()
    }
}

fn font_size(v: &str) -> Option<u8> {
    let v = v.trim();
    let n: i32 = v.trim_start_matches(['+', '-']).parse().ok()?;
    let n = if v.starts_with('+') {
        3 + n
    } else if v.starts_with('-') {
        3 - n
    } else {
        n
    };
    Some(n.clamp(1, 7) as u8)
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_paragraphs_become_headings() {
        let html = r#"<p height="1em" width="0pt" align="center"><font size="6"><b>Chapter One</b></font></p><p width="2em">Text &amp; more <font size="2">small</font></p>"#;
        let out = normalize(html);
        assert!(out.starts_with("<h1 style=\"margin-top:1em;text-indent:0pt;text-align:center\"><span style=\"font-size:2em\"><b>Chapter One</b></span></h1>"), "{out}");
        assert!(out.contains("<p style=\"text-indent:2em\">Text &amp; more <span style=\"font-size:0.82em\">small</span></p>"), "{out}");
    }
}
