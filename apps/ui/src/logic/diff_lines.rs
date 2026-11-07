//! Placeholder for `diff-lines.ts`. The changes slice fills this in.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: String,
    pub text: String,
}

pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let _ = text;
    Vec::new()
}
