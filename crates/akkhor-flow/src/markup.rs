//! A small, forgiving HTML/XHTML tokenizer.
//!
//! EPUB chapters are XHTML, but MOBI, FB2 and real-world EPUBs are often not
//! well-formed, so this never fails: anything it cannot make sense of becomes
//! text. It does not build a tree; the document model consumes tokens
//! directly, which keeps memory flat even for very long chapters.

use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    Start {
        /// Lower-cased local name (`xhtml:p` becomes `p`).
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    End {
        name: String,
    },
    Text(Cow<'a, str>),
}

pub struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    /// Skips past `end`, or to the end of input when it is missing.
    fn skip_past(&mut self, end: &str) -> &'a str {
        let rest = self.rest();
        match rest.find(end) {
            Some(i) => {
                self.pos += i + end.len();
                &rest[..i]
            }
            None => {
                self.pos = self.src.len();
                rest
            }
        }
    }

    fn parse_tag(&mut self) -> Option<Token<'a>> {
        // We are at '<'.
        let rest = self.rest();
        let bytes = rest.as_bytes();
        let closing = bytes.get(1) == Some(&b'/');
        let name_start = if closing { 2 } else { 1 };
        let name_len = rest[name_start..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len() - name_start);
        if name_len == 0 {
            return None;
        }
        let raw_name = &rest[name_start..name_start + name_len];
        if !raw_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            return None;
        }
        let name = local_name(raw_name);

        let tag_end = find_tag_end(rest)?;
        let inner = &rest[name_start + name_len..tag_end];
        self.pos += tag_end + 1;

        if closing {
            return Some(Token::End { name });
        }
        let self_closing = inner.trim_end().ends_with('/');
        Some(Token::Start {
            attrs: parse_attrs(inner.trim_end().trim_end_matches('/')),
            name,
            self_closing,
        })
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let rest = self.rest();
            if rest.is_empty() {
                return None;
            }
            if let Some(after) = rest.strip_prefix('<') {
                if after.starts_with("!--") {
                    self.pos += 4;
                    self.skip_past("-->");
                    continue;
                }
                if after.starts_with("![CDATA[") {
                    self.pos += 9;
                    let text = self.skip_past("]]>");
                    return Some(Token::Text(Cow::Borrowed(text)));
                }
                if after.starts_with('!') || after.starts_with('?') {
                    self.skip_past(">");
                    continue;
                }
                if let Some(tok) = self.parse_tag() {
                    return Some(tok);
                }
                // A stray '<' is text.
                self.pos += 1;
                return Some(Token::Text(Cow::Borrowed("<")));
            }
            let end = rest.find('<').unwrap_or(rest.len());
            self.pos += end;
            return Some(Token::Text(decode_entities(&rest[..end])));
        }
    }
}

fn local_name(raw: &str) -> String {
    raw.rsplit(':').next().unwrap_or(raw).to_ascii_lowercase()
}

/// Index of the '>' that ends the tag, skipping '>' inside quoted values.
fn find_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &b) in s.as_bytes().iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i),
            None => {}
        }
    }
    None
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let name = local_name(&rest[..name_end]);
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after_eq) = rest.strip_prefix('=') {
            let after_eq = after_eq.trim_start();
            let (v, remaining) = match after_eq.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = &after_eq[1..];
                    let end = body.find(q).unwrap_or(body.len());
                    (&body[..end], body.get(end + 1..).unwrap_or(""))
                }
                _ => {
                    let end = after_eq.find(char::is_whitespace).unwrap_or(after_eq.len());
                    (&after_eq[..end], &after_eq[end..])
                }
            };
            value = decode_entities(v).into_owned();
            rest = remaining.trim_start();
        }
        if !name.is_empty() {
            out.push((name, value));
        }
        if name_end == 0 && !rest.is_empty() {
            // Unparseable byte; skip it so we always make progress.
            let skip = rest.chars().next().map_or(1, char::len_utf8);
            rest = &rest[skip..];
        }
    }
    out
}

/// Decodes `&amp;`-style entities. Unknown entities are left as written.
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.as_bytes()[..rest.len().min(12)]
            .iter()
            .position(|&b| b == b';');
        let decoded = end.and_then(|e| decode_entity(&rest[1..e]).map(|c| (c, e)));
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{A0}',
        "shy" => '\u{AD}',
        "zwj" => '\u{200D}',
        "zwnj" => '\u{200C}',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201C}',
        "rdquo" => '\u{201D}',
        "hellip" => '\u{2026}',
        "copy" => '\u{A9}',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_messy_html() {
        let toks: Vec<_> = Tokenizer::new(
            "<?xml version='1.0'?><!-- c --><P class=\"a>b\">আমি &amp; তুমি<br/></p><x:i>x</x:i> < 3",
        )
        .collect();
        assert_eq!(
            toks[0],
            Token::Start {
                name: "p".into(),
                attrs: vec![("class".into(), "a>b".into())],
                self_closing: false
            }
        );
        assert_eq!(toks[1], Token::Text("আমি & তুমি".into()));
        assert!(matches!(&toks[2], Token::Start { name, self_closing: true, .. } if name == "br"));
        assert_eq!(toks[3], Token::End { name: "p".into() });
        assert!(matches!(&toks[4], Token::Start { name, .. } if name == "i"));
        let text: String = toks
            .iter()
            .filter_map(|t| match t {
                Token::Text(t) => Some(t.as_ref()),
                _ => None,
            })
            .collect();
        assert!(text.ends_with(" < 3"));
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode_entities("a&#x9CE;b&#2381;&bogus;"), "aৎb्&bogus;");
    }
}
