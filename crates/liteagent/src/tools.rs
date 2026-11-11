//! Built-in tools used by the demo and the speed test.
//!
//! File tools stay inside the work folder. There is no shell: the Python runtime
//! does not ship one. A host that wants `run` can register its own tool.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::toolpath::{Tool, object_schema};

/// System prompt used by the Python runtime. Tool instructions are appended.
pub const DEFAULT_SYSTEM: &str = "You are a careful assistant that solves tasks step by step using tools. \
Use tools for arithmetic and for anything about files; never guess file contents. \
Keep thoughts to one short sentence.";

const MAX_TOOL_CHARS: usize = 2000;

/// `calculator`, `list_files`, `read_file`, and `write_note`, rooted at `workdir`.
pub fn default_tools(workdir: impl AsRef<Path>) -> Vec<Tool> {
    let root = workdir.as_ref().to_path_buf();
    let string = json!({"type": "string"});
    vec![
        Tool::new(
            "calculator",
            "evaluate arithmetic, e.g. 12*(3+4)",
            object_schema(&[("expression", string.clone())]),
            |args| {
                let expr = args
                    .get("expression")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "bad arguments: expression must be a string".to_string())?;
                Ok(eval_arith(expr))
            },
        ),
        Tool::new(
            "list_files",
            "list files in a folder of the work folder",
            object_schema(&[("folder", string.clone())]),
            {
                let root = root.clone();
                move |args| {
                    let folder = args
                        .get("folder")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "bad arguments: folder must be a string".to_string())?;
                    Ok(list_files(&root, folder))
                }
            },
        ),
        Tool::new(
            "read_file",
            "read a text file from the work folder",
            object_schema(&[("path", string.clone())]),
            {
                let root = root.clone();
                move |args| {
                    let path = args
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "bad arguments: path must be a string".to_string())?;
                    Ok(read_file(&root, path))
                }
            },
        ),
        Tool::new(
            "write_note",
            "write a text file in the work folder",
            object_schema(&[("path", string.clone()), ("text", string)]),
            move |args| {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "bad arguments: path must be a string".to_string())?;
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "bad arguments: text must be a string".to_string())?;
                Ok(write_note(&root, path, text))
            },
        ),
    ]
}

fn list_files(root: &Path, folder: &str) -> String {
    let path = match resolve_under(root, folder) {
        Ok(path) => path,
        Err(err) => return format!("error: {err}"),
    };
    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let mut names = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
                    Err(err) => return format!("error: {err}"),
                }
            }
            names.sort();
            if names.is_empty() {
                "(empty)".to_string()
            } else {
                names.join("\n")
            }
        }
        Err(err) => format!("error: {err}"),
    }
}

fn read_file(root: &Path, path: &str) -> String {
    let path = match resolve_under(root, path) {
        Ok(path) => path,
        Err(err) => return format!("error: {err}"),
    };
    match std::fs::read(&path) {
        Ok(bytes) => trim_output(&String::from_utf8_lossy(&bytes)),
        Err(err) => format!("error: {err}"),
    }
}

fn write_note(root: &Path, path: &str, text: &str) -> String {
    let path = match resolve_under(root, path) {
        Ok(path) => path,
        Err(err) => return format!("error: {err}"),
    };
    match std::fs::write(&path, text.as_bytes()) {
        Ok(()) => "ok".to_string(),
        Err(err) => format!("error: {err}"),
    }
}

fn trim_output(text: &str) -> String {
    let count = text.chars().count();
    if count <= MAX_TOOL_CHARS {
        text.to_string()
    } else {
        let trimmed: String = text.chars().take(MAX_TOOL_CHARS).collect();
        format!("{trimmed}\n...[trimmed]")
    }
}

/// Lexical jail: `..` may not leave `root`. This is component-wise, so
/// `work_evil` is not treated as inside `work`.
fn resolve_under(root: &Path, user_path: &str) -> std::result::Result<PathBuf, String> {
    let root_norm = lexical_normal(root);
    let candidate = Path::new(user_path);
    let joined = if user_path.is_empty() {
        root_norm.clone()
    } else if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let full = lexical_normal(&joined);
    if full != root_norm && !full.starts_with(&root_norm) {
        return Err("path outside the work folder".to_string());
    }
    Ok(full)
}

fn lexical_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

fn eval_arith(expression: &str) -> String {
    match Arith::parse(expression) {
        Ok(num) => format_num(num),
        Err(err) => format!("error: {err}"),
    }
}

#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
}

fn format_num(n: Num) -> String {
    match n {
        Num::Int(i) => i.to_string(),
        Num::Float(f) => format_py_float(f),
    }
}

/// `str(round(v, 6))` from CPython: six decimal places, trailing zeros trimmed,
/// and a float always keeps a decimal point (`2.0`, not `2`).
fn format_py_float(f: f64) -> String {
    if !f.is_finite() {
        return "error: overflow".to_string();
    }
    let scale = 1_000_000.0;
    let rounded = (f * scale).round() / scale;
    let negative = rounded.is_sign_negative() && rounded != 0.0;
    let abs = rounded.abs();
    let raw = format!("{abs:.6}");
    let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
    let body = if trimmed.contains('.') {
        trimmed.to_string()
    } else {
        format!("{trimmed}.0")
    };
    if negative { format!("-{body}") } else { body }
}

fn add(a: Num, b: Num) -> std::result::Result<Num, String> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x
            .checked_add(y)
            .map(Num::Int)
            .ok_or_else(|| "overflow".to_string()),
        (x, y) => Ok(Num::Float(x.as_f64() + y.as_f64())),
    }
}

fn sub(a: Num, b: Num) -> std::result::Result<Num, String> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x
            .checked_sub(y)
            .map(Num::Int)
            .ok_or_else(|| "overflow".to_string()),
        (x, y) => Ok(Num::Float(x.as_f64() - y.as_f64())),
    }
}

fn mul(a: Num, b: Num) -> std::result::Result<Num, String> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x
            .checked_mul(y)
            .map(Num::Int)
            .ok_or_else(|| "overflow".to_string()),
        (x, y) => Ok(Num::Float(x.as_f64() * y.as_f64())),
    }
}

fn div(a: Num, b: Num) -> std::result::Result<Num, String> {
    let d = b.as_f64();
    if d == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Num::Float(a.as_f64() / d))
}

fn floor_div(a: Num, b: Num) -> std::result::Result<Num, String> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => {
            if y == 0 {
                return Err("division by zero".to_string());
            }
            Ok(Num::Int(x.div_euclid(y)))
        }
        (x, y) => {
            let d = y.as_f64();
            if d == 0.0 {
                return Err("division by zero".to_string());
            }
            Ok(Num::Float((x.as_f64() / d).floor()))
        }
    }
}

fn modulo(a: Num, b: Num) -> std::result::Result<Num, String> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => {
            if y == 0 {
                return Err("division by zero".to_string());
            }
            Ok(Num::Int(x.rem_euclid(y)))
        }
        (x, y) => {
            let d = y.as_f64();
            if d == 0.0 {
                return Err("division by zero".to_string());
            }
            Ok(Num::Float(x.as_f64().rem_euclid(d)))
        }
    }
}

fn pow(a: Num, b: Num) -> std::result::Result<Num, String> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) if y >= 0 => {
            let exp = u32::try_from(y).map_err(|_| "overflow".to_string())?;
            x.checked_pow(exp)
                .map(Num::Int)
                .ok_or_else(|| "overflow".to_string())
        }
        (x, y) => {
            let r = x.as_f64().powf(y.as_f64());
            if !r.is_finite() {
                return Err("overflow".to_string());
            }
            Ok(Num::Float(r))
        }
    }
}

fn neg(n: Num) -> std::result::Result<Num, String> {
    match n {
        Num::Int(i) => i
            .checked_neg()
            .map(Num::Int)
            .ok_or_else(|| "overflow".to_string()),
        Num::Float(f) => Ok(Num::Float(-f)),
    }
}

/// `+ - * / ** % //`, parentheses, and numbers. Same precedence as Python.
struct Arith<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl<'a> Arith<'a> {
    fn parse(input: &'a str) -> std::result::Result<Num, String> {
        let mut p = Self {
            s: input.as_bytes(),
            i: 0,
            depth: 0,
        };
        let v = p.expr()?;
        p.skip();
        if p.i != p.s.len() {
            return Err(allowed());
        }
        Ok(v)
    }

    fn expr(&mut self) -> std::result::Result<Num, String> {
        self.enter()?;
        let mut left = self.term()?;
        loop {
            self.skip();
            if self.eat(b'+') {
                let right = self.term()?;
                left = add(left, right)?;
            } else if self.eat(b'-') {
                let right = self.term()?;
                left = sub(left, right)?;
            } else {
                break;
            }
        }
        self.depth -= 1;
        Ok(left)
    }

    fn term(&mut self) -> std::result::Result<Num, String> {
        let mut left = self.factor()?;
        loop {
            self.skip();
            if self.starts(b"//") {
                self.i += 2;
                let right = self.factor()?;
                left = floor_div(left, right)?;
            } else if self.starts(b"**") {
                return Err(allowed());
            } else if self.eat(b'*') {
                let right = self.factor()?;
                left = mul(left, right)?;
            } else if self.eat(b'/') {
                let right = self.factor()?;
                left = div(left, right)?;
            } else if self.eat(b'%') {
                let right = self.factor()?;
                left = modulo(left, right)?;
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn factor(&mut self) -> std::result::Result<Num, String> {
        self.enter()?;
        self.skip();
        let value = if self.eat(b'+') {
            self.factor()
        } else if self.eat(b'-') {
            neg(self.factor()?)
        } else {
            self.power()
        };
        self.depth -= 1;
        value
    }

    fn power(&mut self) -> std::result::Result<Num, String> {
        let base = self.primary()?;
        self.skip();
        if self.starts(b"**") {
            self.i += 2;
            let exp = self.factor()?;
            pow(base, exp)
        } else {
            Ok(base)
        }
    }

    fn primary(&mut self) -> std::result::Result<Num, String> {
        self.skip();
        if self.eat(b'(') {
            let v = self.expr()?;
            self.skip();
            if !self.eat(b')') {
                return Err(allowed());
            }
            return Ok(v);
        }
        self.number()
    }

    fn number(&mut self) -> std::result::Result<Num, String> {
        self.skip();
        let start = self.i;
        let mut saw_digit = false;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            saw_digit = true;
            self.i += 1;
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.i += 1;
            let mut frac = false;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                frac = true;
                self.i += 1;
            }
            if !frac && !saw_digit {
                return Err(allowed());
            }
            saw_digit = true;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let exp_at = self.i;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            if self.i == exp_at {
                return Err(allowed());
            }
        }
        if !saw_digit {
            return Err(allowed());
        }
        let lex = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| allowed())?;
        if is_float {
            let f = lex.parse::<f64>().map_err(|_| allowed())?;
            if !f.is_finite() {
                return Err("overflow".to_string());
            }
            Ok(Num::Float(f))
        } else {
            lex.parse::<i64>()
                .map(Num::Int)
                .map_err(|_| "overflow".to_string())
        }
    }

    fn enter(&mut self) -> std::result::Result<(), String> {
        if self.depth >= 64 {
            return Err(allowed());
        }
        self.depth += 1;
        Ok(())
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn starts(&self, pat: &[u8]) -> bool {
        self.s[self.i..].starts_with(pat)
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
}

fn allowed() -> String {
    "only + - * / ** % // and numbers are allowed".to_string()
}
