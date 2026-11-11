//! Python `json.dumps` / `json.loads` compatible JSON.
//!
//! Assistant messages are re-serialized the way CPython's `json.dumps` does
//! (spaces after `:` and `,`, insertion order, `ensure_ascii`). The llama-server
//! cache test compares those strings character by character. `serde_json::Map`
//! sorts keys, so steps keep their own order here.

use serde_json::{Map, Number, Value};

/// JSON value that remembers object key order and number lexemes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PyVal {
    Null,
    Bool(bool),
    /// Exact number text from the input (`1`, `1.0`, `1e2`).
    Num(String),
    Str(String),
    Arr(Vec<PyVal>),
    Obj(Vec<(String, PyVal)>),
}

const MAX_DEPTH: u32 = 64;

pub(crate) fn parse_json(input: &str) -> Option<PyVal> {
    let mut p = Parser {
        s: input.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.skip();
    if p.i != p.s.len() {
        return None;
    }
    Some(v)
}

pub(crate) fn dump(v: &PyVal, sort_keys: bool) -> String {
    let mut out = String::new();
    write_val(&mut out, v, sort_keys);
    out
}

pub(crate) fn dump_value(v: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, v);
    out
}

pub(crate) fn to_value(v: &PyVal) -> Value {
    match v {
        PyVal::Null => Value::Null,
        PyVal::Bool(b) => Value::Bool(*b),
        PyVal::Num(lex) => number_value(lex),
        PyVal::Str(s) => Value::String(s.clone()),
        PyVal::Arr(items) => Value::Array(items.iter().map(to_value).collect()),
        PyVal::Obj(pairs) => {
            let mut map = Map::new();
            for (k, child) in pairs {
                map.insert(k.clone(), to_value(child));
            }
            Value::Object(map)
        }
    }
}

fn number_value(lex: &str) -> Value {
    if let Ok(i) = lex.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(u) = lex.parse::<u64>() {
        return Value::Number(u.into());
    }
    if let Ok(f) = lex.parse::<f64>()
        && let Some(n) = Number::from_f64(f)
    {
        return Value::Number(n);
    }
    Value::Null
}

fn write_val(out: &mut String, v: &PyVal, sort_keys: bool) {
    match v {
        PyVal::Null => out.push_str("null"),
        PyVal::Bool(true) => out.push_str("true"),
        PyVal::Bool(false) => out.push_str("false"),
        PyVal::Num(lex) => out.push_str(lex),
        PyVal::Str(s) => push_string(out, s),
        PyVal::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_val(out, item, sort_keys);
            }
            out.push(']');
        }
        PyVal::Obj(pairs) => {
            let mut owned;
            let pairs_ref: &[(String, PyVal)] = if sort_keys {
                owned = pairs.clone();
                owned.sort_by(|a, b| a.0.cmp(&b.0));
                &owned
            } else {
                pairs
            };
            out.push('{');
            for (i, (k, child)) in pairs_ref.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                push_string(out, k);
                out.push_str(": ");
                write_val(out, child, sort_keys);
            }
            out.push('}');
        }
    }
}

fn write_json(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => push_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, child)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                push_string(out, k);
                out.push_str(": ");
                write_json(out, child);
            }
            out.push('}');
        }
    }
}

fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if (c as u32) > 0x7f => {
                let u = c as u32;
                if u > 0xffff {
                    let v = u - 0x10000;
                    let hi = 0xD800 + (v >> 10);
                    let lo = 0xDC00 + (v & 0x3ff);
                    out.push_str(&format!("\\u{hi:04x}\\u{lo:04x}"));
                } else {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn value(&mut self) -> Option<PyVal> {
        self.skip();
        if self.depth >= MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'n' => self.eat_lit(b"null").then_some(PyVal::Null),
            b't' => self.eat_lit(b"true").then_some(PyVal::Bool(true)),
            b'f' => self.eat_lit(b"false").then_some(PyVal::Bool(false)),
            b'"' => self.string().map(PyVal::Str),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn array(&mut self) -> Option<PyVal> {
        self.bump(b'[')?;
        self.depth += 1;
        let mut items = Vec::new();
        self.skip();
        if self.peek() == Some(b']') {
            self.i += 1;
            self.depth -= 1;
            return Some(PyVal::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip();
            match self.peek()? {
                b']' => {
                    self.i += 1;
                    self.depth -= 1;
                    return Some(PyVal::Arr(items));
                }
                b',' => self.i += 1,
                _ => return None,
            }
        }
    }

    fn object(&mut self) -> Option<PyVal> {
        self.bump(b'{')?;
        self.depth += 1;
        let mut pairs = Vec::new();
        self.skip();
        if self.peek() == Some(b'}') {
            self.i += 1;
            self.depth -= 1;
            return Some(PyVal::Obj(pairs));
        }
        loop {
            self.skip();
            if self.peek() != Some(b'"') {
                return None;
            }
            let key = self.string()?;
            self.skip();
            self.bump(b':')?;
            let val = self.value()?;
            pairs.push((key, val));
            self.skip();
            match self.peek()? {
                b'}' => {
                    self.i += 1;
                    self.depth -= 1;
                    return Some(PyVal::Obj(pairs));
                }
                b',' => self.i += 1,
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<PyVal> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        if self.peek() == Some(b'0') {
            self.i += 1;
        } else if matches!(self.peek(), Some(b'1'..=b'9')) {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        } else {
            return None;
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        let lex = std::str::from_utf8(&self.s[start..self.i]).ok()?;
        Some(PyVal::Num(lex.to_string()))
    }

    fn string(&mut self) -> Option<String> {
        self.bump(b'"')?;
        let mut out = String::new();
        while let Some(b) = self.peek() {
            self.i += 1;
            match b {
                b'"' => return Some(out),
                b'\\' => {
                    let esc = self.peek()?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode()?),
                        _ => return None,
                    }
                }
                c if c < 0x20 => return None,
                c => {
                    if c < 0x80 {
                        out.push(c as char);
                    } else {
                        self.i -= 1;
                        let rest = std::str::from_utf8(&self.s[self.i..]).ok()?;
                        let ch = rest.chars().next()?;
                        self.i += ch.len_utf8();
                        out.push(ch);
                    }
                }
            }
        }
        None
    }

    fn unicode(&mut self) -> Option<char> {
        let unit = self.hex4()?;
        let u = if (0xD800..=0xDBFF).contains(&unit) {
            if self.peek() != Some(b'\\') {
                return char::from_u32(0xFFFD);
            }
            self.i += 1;
            if self.peek() != Some(b'u') {
                return None;
            }
            self.i += 1;
            let lo = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&lo) {
                return char::from_u32(0xFFFD);
            }
            0x10000 + (((unit - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            return char::from_u32(0xFFFD);
        } else {
            unit as u32
        };
        char::from_u32(u)
    }

    fn hex4(&mut self) -> Option<u16> {
        let mut n = 0u16;
        for _ in 0..4 {
            let h = self.peek()?;
            self.i += 1;
            let d = match h {
                b'0'..=b'9' => h - b'0',
                b'a'..=b'f' => h - b'a' + 10,
                b'A'..=b'F' => h - b'A' + 10,
                _ => return None,
            };
            n = (n << 4) | u16::from(d);
        }
        Some(n)
    }

    fn eat_lit(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn bump(&mut self, b: u8) -> Option<()> {
        if self.peek() == Some(b) {
            self.i += 1;
            Some(())
        } else {
            None
        }
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
}

#[cfg(test)]
mod pyjson_tests {
    use super::*;

    #[test]
    fn roundtrip_python_step() {
        let raw =
            r#"{"thought": "compute", "tool": "calculator", "args": {"expression": "2340*0.175"}}"#;
        let v = parse_json(raw).expect("parse");
        assert_eq!(dump(&v, false), raw);
    }

    #[test]
    fn sort_keys_orders_nested_objects() {
        let v = parse_json(r#"{"b": 1, "a": {"d": 2, "c": 3}}"#).expect("parse");
        assert_eq!(dump(&v, true), r#"{"a": {"c": 3, "d": 2}, "b": 1}"#);
    }

    #[test]
    fn ensure_ascii_escapes_non_ascii() {
        let v = PyVal::Str("é".to_string());
        assert_eq!(dump(&v, false), r#""\u00e9""#);
    }
}
