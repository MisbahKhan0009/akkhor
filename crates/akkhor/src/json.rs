//! A tiny JSON writer for results handed across the app boundary (the JNI
//! bridge passes one string per call). Only what the results need: objects
//! with string, number, bool, null and array values, written in order.

use std::fmt::Write;

/// A JSON object under construction.
#[derive(Default)]
pub struct Obj {
    out: String,
}

/// Writes `s` as a JSON string literal.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Obj {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(&mut self, k: &str) {
        self.out.push(if self.out.is_empty() { '{' } else { ',' });
        self.out.push_str(&string(k));
        self.out.push(':');
    }

    pub fn str(mut self, k: &str, v: &str) -> Self {
        self.key(k);
        self.out.push_str(&string(v));
        self
    }

    pub fn opt_str(mut self, k: &str, v: Option<&str>) -> Self {
        self.key(k);
        match v {
            Some(v) => self.out.push_str(&string(v)),
            None => self.out.push_str("null"),
        }
        self
    }

    pub fn num(mut self, k: &str, v: f64) -> Self {
        self.key(k);
        if v.is_finite() {
            // Integers without a fraction; others to 4 places.
            if v.fract() == 0.0 && v.abs() < 1e15 {
                let _ = write!(self.out, "{}", v as i64);
            } else {
                let _ = write!(self.out, "{:.4}", v);
            }
        } else {
            self.out.push_str("null");
        }
        self
    }

    pub fn opt_num(self, k: &str, v: Option<f64>) -> Self {
        match v {
            Some(v) => self.num(k, v),
            None => self.raw(k, "null"),
        }
    }

    pub fn bool(self, k: &str, v: bool) -> Self {
        self.raw(k, if v { "true" } else { "false" })
    }

    pub fn strs(mut self, k: &str, v: &[String]) -> Self {
        self.key(k);
        self.out.push('[');
        for (i, s) in v.iter().enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            self.out.push_str(&string(s));
        }
        self.out.push(']');
        self
    }

    /// A value already written as JSON (an object, an array).
    pub fn raw(mut self, k: &str, json: &str) -> Self {
        self.key(k);
        self.out.push_str(json);
        self
    }

    pub fn end(mut self) -> String {
        if self.out.is_empty() {
            self.out.push('{');
        }
        self.out.push('}');
        self.out
    }
}

/// A JSON array of values already written as JSON.
pub fn array<I: IntoIterator<Item = String>>(items: I) -> String {
    let mut out = String::from("[");
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&item);
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_objects() {
        let j = Obj::new()
            .str("title", "নদী \"ও\" গ্রাম\n")
            .opt_str("author", None)
            .num("pages", 12.0)
            .num("quality", 0.5)
            .bool("ok", true)
            .strs("authors", &["এক".into(), "দুই".into()])
            .end();
        assert_eq!(
            j,
            r#"{"title":"নদী \"ও\" গ্রাম\n","author":null,"pages":12,"quality":0.5000,"ok":true,"authors":["এক","দুই"]}"#
        );
        assert_eq!(Obj::new().end(), "{}");
        assert_eq!(array(["1".into(), "{}".into()]), "[1,{}]");
    }
}
