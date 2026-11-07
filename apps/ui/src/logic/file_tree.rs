//! Placeholder for `file-tree.ts`. The files slice fills this in.

pub const FILE_ROW_HEIGHT: i64 = 24;
pub const FILE_WINDOW_THRESHOLD: i64 = 500;
pub const FILE_LINE_CAP: i64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub status: String,
}

pub fn build_tree(entries: &[FileEntry]) -> Vec<String> {
    entries.iter().map(|entry| entry.path.clone()).collect()
}

pub fn format_bytes(bytes: u64) -> String {
    format!("{bytes}")
}
