//! The few XMP (RDF/XML) fields a library card needs, read without an XML
//! parser: the text of `rdf:li` items (or the element itself) under a
//! property such as `dc:title`.

/// The first value of `property` (for `dc:title`, the default language).
pub(crate) fn first(xmp: &str, property: &str) -> Option<String> {
    let all = all(xmp, property);
    // A title is an rdf:Alt: prefer x-default, else the first.
    let body = inside(xmp, property)?;
    if let Some(i) = body.find("xml:lang=\"x-default\"")
        && let Some(v) = item_after(&body[i..])
    {
        return Some(v);
    }
    all.into_iter().next()
}

/// Every value of `property` (for `dc:creator`, each author).
pub(crate) fn all(xmp: &str, property: &str) -> Vec<String> {
    let Some(body) = inside(xmp, property) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("<rdf:li") {
        rest = &rest[i..];
        match item_after(rest) {
            Some(v) => out.push(v),
            None => break,
        }
        rest = &rest[7..];
    }
    if out.is_empty() {
        // A plain property: <dc:title>Text</dc:title>.
        let v = unescape(body.trim());
        if !v.is_empty() && !v.contains('<') {
            out.push(v);
        }
    }
    out
}

/// The text between `<property ...>` and `</property>`.
fn inside<'a>(xmp: &'a str, property: &str) -> Option<&'a str> {
    let open = xmp.find(&format!("<{property}"))?;
    let after = &xmp[open + property.len() + 1..];
    // `<dc:title/>` or `<dc:titleX`: not this property.
    if !after.starts_with(['>', ' ', '\n', '\r', '\t']) {
        return None;
    }
    let start = after.find('>')? + 1;
    if after[..start].ends_with("/>") {
        return None;
    }
    let end = after[start..].find(&format!("</{property}>"))?;
    Some(&after[start..start + end])
}

/// The text of the `rdf:li` element at the start of `s`.
fn item_after(s: &str) -> Option<String> {
    let start = s.find('>')? + 1;
    if s[..start].ends_with("/>") {
        return None;
    }
    let end = s[start..].find("</rdf:li>")?;
    let v = unescape(s[start..start + end].trim());
    (!v.is_empty()).then_some(v)
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // Bytes, not text: 12 may fall inside a letter (`&` before Bengali).
        let Some(end) = rest.as_bytes()[..rest.len().min(12)]
            .iter()
            .position(|&b| b == b';')
        else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_ampersand_before_letters() {
        // Found by the same sweep as an FB2 fuzz crash: the entity search
        // cut the text 12 bytes on, inside a Bengali letter.
        assert_eq!(unescape("নদী&গ্রাম"), "নদী&গ্রাম");
        assert_eq!(unescape("&বাংলা &amp; ই"), "&বাংলা & ই");
    }

    const XMP: &str = r#"<x:xmpmeta><rdf:RDF><rdf:Description rdf:about=""
        xmlns:dc="http://purl.org/dc/elements/1.1/">
      <dc:title><rdf:Alt>
        <rdf:li xml:lang="en">An English title</rdf:li>
        <rdf:li xml:lang="x-default">নদী &amp; গ্রাম</rdf:li>
      </rdf:Alt></dc:title>
      <dc:creator><rdf:Seq><rdf:li>রবীন্দ্রনাথ ঠাকুর</rdf:li><rdf:li>A. &#x4E;ame</rdf:li></rdf:Seq></dc:creator>
      <dc:format>application/pdf</dc:format>
    </rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn reads_titles_and_creators() {
        assert_eq!(first(XMP, "dc:title").as_deref(), Some("নদী & গ্রাম"));
        assert_eq!(all(XMP, "dc:creator"), ["রবীন্দ্রনাথ ঠাকুর", "A. Name"]);
        assert_eq!(first(XMP, "dc:format").as_deref(), Some("application/pdf"));
        assert_eq!(first(XMP, "dc:subject"), None);
        assert_eq!(first("<dc:title/>", "dc:title"), None);
        assert_eq!(first("<dc:title>a &bogus; b", "dc:title"), None);
        assert_eq!(unescape("a &bogus; & b &#2453;"), "a &bogus; & b ক");
    }
}
