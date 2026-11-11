use serde_json::{Map, Value};

/// Lines added and removed, before they are attached to a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineChange {
    pub added: i64,
    pub removed: i64,
}

/// Number of lines in a string. A trailing newline does not add an extra line.
pub fn line_count(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    let mut count = 0i64;
    let mut rest = text;
    loop {
        match rest.find('\n') {
            Some(index) => {
                count = count.saturating_add(1);
                rest = &rest[index + 1..];
                if rest.is_empty() {
                    return count;
                }
            }
            None => return count.saturating_add(1),
        }
    }
}

/// Edit: lines of `new_text` versus `old_text`.
pub fn edit_size(old_text: &str, new_text: &str) -> LineChange {
    LineChange {
        added: line_count(new_text),
        removed: line_count(old_text),
    }
}

pub fn write_size(content: &str) -> LineChange {
    LineChange {
        added: line_count(content),
        removed: 0,
    }
}

pub fn sum_edit_size(parts: &[LineChange]) -> LineChange {
    let mut added = 0i64;
    let mut removed = 0i64;
    for part in parts {
        added = added.saturating_add(part.added);
        removed = removed.saturating_add(part.removed);
    }
    LineChange { added, removed }
}

/// Counts a completed Cursor edit or write reports, when present.
pub fn reported_edit_size(value: &Value) -> Option<LineChange> {
    let obj = value.as_object()?;
    let source = match obj.get("success") {
        Some(Value::Object(success)) => success,
        _ => obj,
    };
    let added = pick_count(source, &["added", "addedLines", "insertions", "linesAdded"])?;
    let removed = pick_count(
        source,
        &["removed", "removedLines", "deletions", "linesRemoved"],
    )?;
    Some(LineChange { added, removed })
}

/// Path relative to cwd when it is inside it, with forward slashes.
pub fn relative_to_cwd(file_path: &str, cwd: Option<&str>) -> String {
    let file = file_path.replace('\\', "/");
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return file;
    };
    let mut root = cwd.replace('\\', "/");
    if root.ends_with('/') {
        root.pop();
    }
    if file == root {
        return ".".to_string();
    }
    let prefix = format!("{root}/");
    if let Some(rest) = file.strip_prefix(&prefix) {
        return rest.to_string();
    }
    if let Some(rest) = strip_prefix_ignore_case(&file, &prefix) {
        return rest;
    }
    file
}

pub fn cap_detail(text: &str) -> &str {
    const MAX: usize = 4000;
    let count = text.chars().count();
    if count <= MAX {
        return text;
    }
    let skip = count - MAX;
    let start = text
        .char_indices()
        .nth(skip)
        .map(|(index, _)| index)
        .unwrap_or(0);
    &text[start..]
}

pub fn first_line(text: &str) -> &str {
    first_line_max(text, 120)
}

pub fn first_line_max(text: &str, max: usize) -> &str {
    let head = match text.find('\n') {
        Some(index) => {
            let head = &text[..index];
            head.strip_suffix('\r').unwrap_or(head)
        }
        None => text,
    };
    let head = head.trim_end();
    if head.chars().count() <= max {
        return head;
    }
    let mut end = head.len();
    for (seen, (index, _)) in head.char_indices().enumerate() {
        if seen == max {
            end = index;
            break;
        }
    }
    &head[..end]
}

fn pick_count(obj: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for key in keys {
        let Some(number) = obj.get(*key).and_then(|value| value.as_f64()) else {
            continue;
        };
        if number.is_finite() && number >= 0.0 && number <= i64::MAX as f64 {
            return Some(number.trunc() as i64);
        }
    }
    None
}

fn strip_prefix_ignore_case(file: &str, prefix: &str) -> Option<String> {
    let file_chars: Vec<char> = file.chars().collect();
    let prefix_chars: Vec<char> = prefix.chars().collect();
    if file_chars.len() < prefix_chars.len() {
        return None;
    }
    for (left, right) in file_chars.iter().zip(&prefix_chars) {
        if !chars_eq_ignore_case(*left, *right) {
            return None;
        }
    }
    Some(file_chars[prefix_chars.len()..].iter().collect())
}

fn chars_eq_ignore_case(left: char, right: char) -> bool {
    if left == right || left.eq_ignore_ascii_case(&right) {
        return true;
    }
    let mut left_lower = left.to_lowercase();
    let mut right_lower = right.to_lowercase();
    loop {
        match (left_lower.next(), right_lower.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
    }
}
