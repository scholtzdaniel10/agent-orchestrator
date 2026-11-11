//! Byte-oriented path decoding for git `-z` output.
//!
//! Paths stay as raw bytes until each record is split. Valid UTF-8 is copied
//! through. A byte that is not valid UTF-8 becomes the Latin-1 character for
//! that byte, which keeps the value instead of substituting U+FFFD.

use std::path::{Path, PathBuf};

/// Turn one git path field into a `String` without lossy UTF-8 replacement.
pub(crate) fn git_path(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                out.push_str(text);
                break;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if let Ok(text) = std::str::from_utf8(&rest[..valid]) {
                    out.push_str(text);
                }
                let bad = err.error_len().unwrap_or(1);
                let end = (valid + bad).min(rest.len());
                for &byte in &rest[valid..end] {
                    out.push(char::from(byte));
                }
                rest = &rest[end..];
            }
        }
    }
    out
}

pub(crate) fn read_until_nul(bytes: &[u8], start: usize) -> (&[u8], usize) {
    if start >= bytes.len() {
        return (&[], bytes.len());
    }
    match bytes[start..].iter().position(|&byte| byte == 0) {
        Some(rel) => (&bytes[start..start + rel], start + rel + 1),
        None => (&bytes[start..], bytes.len()),
    }
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

pub(crate) fn utf16_prefix(text: &str, max_units: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        let units = ch.len_utf16();
        if count + units > max_units {
            break;
        }
        out.push(ch);
        count += units;
    }
    out
}

pub(crate) fn utf16_suffix(text: &str, max_units: usize) -> String {
    let mut count = 0usize;
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        let units = ch.len_utf16();
        if count + units > max_units {
            break;
        }
        count += units;
        start = idx;
    }
    text[start..].to_string()
}

pub(crate) fn nonempty_line(text: &str) -> Option<&str> {
    text.lines().find(|line| !line.trim().is_empty())
}

pub(crate) fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

pub(crate) fn canonicalize_path(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok().map(strip_verbatim)
}

pub(crate) fn normalize_abs(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn fold_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        text.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        text.into_owned()
    }
}

/// `equal_ok` is the files-module rule: the root itself counts as inside.
pub(crate) fn is_inside(root: &Path, target: &Path, equal_ok: bool) -> bool {
    let base = fold_path(root);
    let child = fold_path(target);
    if equal_ok && child == base {
        return true;
    }
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let boundary = if base.ends_with(sep) {
        base
    } else {
        format!("{base}{sep}")
    };
    child.starts_with(&boundary)
}

pub(crate) fn slash(path: &str) -> String {
    path.replace('\\', "/")
}

pub(crate) const DIFF_CAP: usize = 400_000;
pub(crate) const DIFF_NOTE: &str = "\n\u{2026} diff cut at 400000 characters";

/// Worktree diff cap: the returned text is at most [`DIFF_CAP`] UTF-16 units.
pub(crate) fn cap_diff(text: &str) -> String {
    let note_len = utf16_len(DIFF_NOTE);
    if utf16_len(text) <= DIFF_CAP {
        return text.to_string();
    }
    let keep = DIFF_CAP.saturating_sub(note_len);
    format!("{}{DIFF_NOTE}", utf16_prefix(text, keep))
}

/// History file diff cap: the note is appended after the first [`DIFF_CAP`] units.
pub(crate) fn cap_show_diff(text: &str) -> String {
    if utf16_len(text) <= DIFF_CAP {
        return text.to_string();
    }
    format!(
        "{}\n\u{2026} diff cut at {DIFF_CAP} characters",
        utf16_prefix(text, DIFF_CAP)
    )
}
