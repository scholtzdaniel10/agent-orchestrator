use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use legate_core::{JobAccess, RunOptions};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|text| !text.is_empty())
}

/// JavaScript `String.prototype.trim`, plus a leading BOM.
pub(crate) fn trim_line(line: &str) -> &str {
    line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

pub(crate) fn path_string(path: &Path) -> String {
    match path.to_str() {
        Some(text) => text.to_string(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// `RunOptions` is typed. A set `access` wins; `edit: true` with no access is edit.
pub(crate) fn job_access(opts: &RunOptions) -> JobAccess {
    if let Some(access) = opts.access {
        return access;
    }
    if opts.edit == Some(true) {
        JobAccess::Edit
    } else {
        JobAccess::Read
    }
}
