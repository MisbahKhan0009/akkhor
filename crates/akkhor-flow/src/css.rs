//! The small part of CSS that matters for reading.
//!
//! Supported selectors: element, `.class`, `#id`, compounds such as
//! `p.first`, descendant chains (`blockquote p`; `>` is treated as a
//! descendant), and comma lists. Anything else (attribute selectors,
//! pseudo-classes, `@media`) is skipped without affecting the rest.
//!
//! Supported properties: `text-align`, `text-indent`, `margin`,
//! `margin-top`, `margin-bottom`, `font-size`, `font-weight`,
//! `font-style`, `display: none`, `page-break-before` / `break-before`.

/// A length in ems of the body font (pixels are taken at 16 px = 1 em).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Em(f32),
    /// A percentage, for `font-size` (relative to the parent).
    Percent(f32),
}

impl Length {
    pub fn em(self) -> f32 {
        match self {
            Length::Em(v) => v,
            Length::Percent(p) => p / 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Justify,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    TextAlign(Align),
    TextIndent(f32),
    MarginTop(f32),
    MarginBottom(f32),
    /// A factor relative to the parent's size.
    FontSize(f32),
    Bold(bool),
    Italic(bool),
    DisplayNone(bool),
    PageBreakBefore(bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Compound {
    element: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

#[derive(Clone, Debug)]
struct Rule {
    /// Compounds from outermost ancestor to the element itself.
    chain: Vec<Compound>,
    specificity: (u32, u32, u32),
    order: usize,
    decls: Vec<Decl>,
}

/// The selector-relevant facts about one element.
#[derive(Clone, Debug, Default)]
pub struct ElementInfo {
    pub name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    rules: Vec<Rule>,
}

impl Stylesheet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(css: &str) -> Self {
        let mut sheet = Self::new();
        sheet.add(css);
        sheet
    }

    /// Adds the rules of another stylesheet (later rules win ties).
    pub fn add(&mut self, css: &str) {
        let css = strip_comments(css);
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let prelude = rest[..open].trim();
            let Some(close) = matching_brace(&rest[open..]).map(|c| open + c) else {
                break;
            };
            let body = &rest[open + 1..close];
            rest = &rest[close + 1..];
            if prelude.starts_with('@') {
                continue; // @media, @font-face, @page: not used
            }
            let decls = parse_declarations(body);
            if decls.is_empty() {
                continue;
            }
            for sel in prelude.split(',') {
                if let Some((chain, specificity)) = parse_selector(sel.trim()) {
                    let order = self.rules.len();
                    self.rules.push(Rule {
                        chain,
                        specificity,
                        order,
                        decls: decls.clone(),
                    });
                }
            }
        }
    }

    /// The declarations that apply to `element` (with `ancestors`, outermost
    /// first), in cascade order: later entries override earlier ones.
    pub fn matching(&self, element: &ElementInfo, ancestors: &[ElementInfo]) -> Vec<Decl> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| chain_matches(&r.chain, element, ancestors))
            .collect();
        hits.sort_by_key(|r| (r.specificity, r.order));
        hits.into_iter()
            .flat_map(|r| r.decls.iter().cloned())
            .collect()
    }
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = rest[i + 2..]
            .find("*/")
            .map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    out.push_str(rest);
    out
}

/// Index of the '}' matching the '{' at the start of `s`.
fn matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_compound(s: &str) -> Option<Compound> {
    if s.is_empty() || s.contains(['[', ':', '*', '+', '~']) {
        return None;
    }
    let mut c = Compound::default();
    let mut rest = s;
    let first_marker = rest.find(['.', '#']).unwrap_or(rest.len());
    if first_marker > 0 {
        c.element = Some(rest[..first_marker].to_ascii_lowercase());
    }
    rest = &rest[first_marker..];
    while !rest.is_empty() {
        let kind = rest.as_bytes()[0];
        let end = rest[1..].find(['.', '#']).map_or(rest.len(), |e| e + 1);
        let name = rest[1..end].to_string();
        if name.is_empty() {
            return None;
        }
        if kind == b'.' {
            c.classes.push(name);
        } else {
            c.id = Some(name);
        }
        rest = &rest[end..];
    }
    Some(c)
}

fn parse_selector(sel: &str) -> Option<(Vec<Compound>, (u32, u32, u32))> {
    let mut chain = Vec::new();
    for part in sel.split(|c: char| c.is_whitespace() || c == '>') {
        if part.is_empty() {
            continue;
        }
        chain.push(parse_compound(part)?);
    }
    if chain.is_empty() {
        return None;
    }
    let mut spec = (0, 0, 0);
    for c in &chain {
        spec.0 += c.id.is_some() as u32;
        spec.1 += c.classes.len() as u32;
        spec.2 += c.element.is_some() as u32;
    }
    Some((chain, spec))
}

fn compound_matches(c: &Compound, e: &ElementInfo) -> bool {
    c.element.as_ref().is_none_or(|n| *n == e.name)
        && c.id.as_ref().is_none_or(|id| e.id.as_ref() == Some(id))
        && c.classes.iter().all(|cl| e.classes.contains(cl))
}

fn chain_matches(chain: &[Compound], element: &ElementInfo, ancestors: &[ElementInfo]) -> bool {
    let Some((last, outer)) = chain.split_last() else {
        return false;
    };
    if !compound_matches(last, element) {
        return false;
    }
    // Match the remaining compounds against ancestors, innermost first.
    let mut anc = ancestors.iter().rev();
    'outer: for c in outer.iter().rev() {
        for a in anc.by_ref() {
            if compound_matches(c, a) {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

/// Parses a length in em; pixels at 16 px per em, points at 12 pt per em.
fn parse_length(v: &str) -> Option<Length> {
    let v = v.trim();
    if v == "0" {
        return Some(Length::Em(0.0));
    }
    let num_end = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(v.len());
    let n: f32 = v[..num_end].parse().ok()?;
    Some(match v[num_end..].trim().to_ascii_lowercase().as_str() {
        "em" | "rem" => Length::Em(n),
        "px" => Length::Em(n / 16.0),
        "pt" => Length::Em(n / 12.0),
        "%" => Length::Percent(n),
        "" => Length::Em(n / 16.0),
        _ => return None,
    })
}

fn parse_font_size(v: &str) -> Option<f32> {
    Some(match v.trim().to_ascii_lowercase().as_str() {
        "xx-small" => 0.6,
        "x-small" => 0.75,
        "small" => 0.89,
        "medium" => 1.0,
        "large" => 1.2,
        "x-large" => 1.5,
        "xx-large" => 2.0,
        "smaller" => 0.83,
        "larger" => 1.2,
        other => parse_length(other)?.em(),
    })
}

/// Parses `prop: value; ...` (a rule body or a `style` attribute).
pub fn parse_declarations(body: &str) -> Vec<Decl> {
    let mut out = Vec::new();
    for decl in body.split(';') {
        let Some((prop, value)) = decl.split_once(':') else {
            continue;
        };
        let prop = prop.trim().to_ascii_lowercase();
        let value = value.trim().trim_end_matches("!important").trim();
        let lower = value.to_ascii_lowercase();
        match prop.as_str() {
            "text-align" => {
                let a = match lower.as_str() {
                    "left" | "start" => Align::Start,
                    "right" | "end" => Align::End,
                    "center" => Align::Center,
                    "justify" => Align::Justify,
                    _ => continue,
                };
                out.push(Decl::TextAlign(a));
            }
            "text-indent" => {
                if let Some(Length::Em(v)) = parse_length(value) {
                    out.push(Decl::TextIndent(v));
                }
            }
            "margin-top" => {
                if let Some(Length::Em(v)) = parse_length(value) {
                    out.push(Decl::MarginTop(v));
                }
            }
            "margin-bottom" => {
                if let Some(Length::Em(v)) = parse_length(value) {
                    out.push(Decl::MarginBottom(v));
                }
            }
            "margin" => {
                let parts: Vec<&str> = value.split_whitespace().collect();
                let (top, bottom) = match parts.len() {
                    1 => (parts[0], parts[0]),
                    2 => (parts[0], parts[0]),
                    3 | 4 => (parts[0], parts[2]),
                    _ => continue,
                };
                if let Some(Length::Em(v)) = parse_length(top) {
                    out.push(Decl::MarginTop(v));
                }
                if let Some(Length::Em(v)) = parse_length(bottom) {
                    out.push(Decl::MarginBottom(v));
                }
            }
            "font-size" => {
                if let Some(f) = parse_font_size(value) {
                    out.push(Decl::FontSize(f));
                }
            }
            "font-weight" => {
                let bold = match lower.as_str() {
                    "bold" | "bolder" => true,
                    "normal" | "lighter" => false,
                    n => match n.parse::<u32>() {
                        Ok(w) => w >= 600,
                        Err(_) => continue,
                    },
                };
                out.push(Decl::Bold(bold));
            }
            "font-style" => out.push(Decl::Italic(lower == "italic" || lower == "oblique")),
            "display" => out.push(Decl::DisplayNone(lower == "none")),
            "page-break-before" | "break-before" => out.push(Decl::PageBreakBefore(matches!(
                lower.as_str(),
                "always" | "page" | "left" | "right"
            ))),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(name: &str, classes: &[&str], id: Option<&str>) -> ElementInfo {
        ElementInfo {
            name: name.into(),
            id: id.map(Into::into),
            classes: classes.iter().map(|c| c.to_string()).collect(),
        }
    }

    #[test]
    fn cascade_by_specificity_and_order() {
        let sheet = Stylesheet::parse(
            "/* c */ p { text-align: justify; text-indent: 1.5em }
             .center { text-align: center }
             p.first { text-indent: 0 }
             @media print { p { text-align: right } }
             blockquote p { font-style: italic }
             a:hover, p[x] { color: red }
             h1 { font-size: 150%; margin: 2em 0 1em }",
        );
        let p = el("p", &["center", "first"], None);
        let d = sheet.matching(&p, &[]);
        assert_eq!(d.last(), Some(&Decl::TextIndent(0.0)));
        assert!(d.contains(&Decl::TextAlign(Align::Center)));
        // center wins over justify: class beats element
        let aligns: Vec<_> = d
            .iter()
            .filter(|x| matches!(x, Decl::TextAlign(_)))
            .collect();
        assert_eq!(aligns.last(), Some(&&Decl::TextAlign(Align::Center)));

        let quoted = sheet.matching(
            &el("p", &[], None),
            &[el("body", &[], None), el("blockquote", &[], None)],
        );
        assert!(quoted.contains(&Decl::Italic(true)));

        let h1 = sheet.matching(&el("h1", &[], None), &[]);
        assert!(h1.contains(&Decl::FontSize(1.5)));
        assert!(h1.contains(&Decl::MarginTop(2.0)));
        assert!(h1.contains(&Decl::MarginBottom(1.0)));
    }

    #[test]
    fn inline_declarations() {
        let d = parse_declarations(
            "font-weight:700; display:none; font-size: 24px; page-break-before: always",
        );
        assert_eq!(
            d,
            vec![
                Decl::Bold(true),
                Decl::DisplayNone(true),
                Decl::FontSize(1.5),
                Decl::PageBreakBefore(true)
            ]
        );
    }
}
