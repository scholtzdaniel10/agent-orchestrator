//! Folder tree for the files panel. Port of `file-tree.ts`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use legate_core::{FileEntry, FileStatus};

pub const FILE_ROW_HEIGHT: i64 = 24;
pub const FILE_WINDOW_THRESHOLD: i64 = 500;
pub const FILE_LINE_CAP: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Dir,
    File,
}

impl RowKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeNode {
    Dir {
        name: String,
        path: String,
        status: FileStatus,
        children: Vec<TreeNode>,
    },
    File {
        name: String,
        path: String,
        status: FileStatus,
    },
}

impl TreeNode {
    pub fn kind(&self) -> RowKind {
        match self {
            Self::Dir { .. } => RowKind::Dir,
            Self::File { .. } => RowKind::File,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Dir { name, .. } | Self::File { name, .. } => name,
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Dir { path, .. } | Self::File { path, .. } => path,
        }
    }

    pub fn status(&self) -> FileStatus {
        match self {
            Self::Dir { status, .. } | Self::File { status, .. } => *status,
        }
    }

    fn children(&self) -> &[TreeNode] {
        match self {
            Self::Dir { children, .. } => children,
            Self::File { .. } => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleRow {
    pub kind: RowKind,
    pub name: String,
    pub path: String,
    pub status: FileStatus,
    pub depth: i64,
    pub has_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRange {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileCounts {
    pub shown: usize,
    pub total: usize,
}

const fn status_rank(status: FileStatus) -> u8 {
    match status {
        FileStatus::Clean => 0,
        FileStatus::Untracked => 1,
        FileStatus::Added => 2,
        FileStatus::Renamed => 3,
        FileStatus::Modified => 4,
        FileStatus::Deleted => 5,
        FileStatus::Conflicted => 6,
    }
}

pub fn status_mark(status: FileStatus) -> Option<&'static str> {
    match status {
        FileStatus::Clean => None,
        FileStatus::Modified => Some("M"),
        FileStatus::Added => Some("A"),
        FileStatus::Untracked => Some("U"),
        FileStatus::Deleted => Some("D"),
        FileStatus::Renamed => Some("R"),
        FileStatus::Conflicted => Some("C"),
    }
}

pub fn status_name(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Clean => "clean",
        FileStatus::Modified => "modified",
        FileStatus::Added => "added",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::Untracked => "untracked",
        FileStatus::Conflicted => "conflicted",
    }
}

fn cmp_name(a: &str, b: &str) -> Ordering {
    let left = a.to_lowercase();
    let right = b.to_lowercase();
    left.cmp(&right).then_with(|| a.cmp(b))
}

fn cmp_node(a: &TreeNode, b: &TreeNode) -> Ordering {
    match (a.kind(), b.kind()) {
        (RowKind::Dir, RowKind::File) => Ordering::Less,
        (RowKind::File, RowKind::Dir) => Ordering::Greater,
        _ => cmp_name(a.name(), b.name()),
    }
}

fn stronger(a: FileStatus, b: FileStatus) -> FileStatus {
    if status_rank(a) >= status_rank(b) {
        a
    } else {
        b
    }
}

fn folder_status(children: &[TreeNode]) -> FileStatus {
    children.iter().fold(FileStatus::Clean, |status, child| {
        stronger(status, child.status())
    })
}

fn file_name(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[index + 1..].to_string(),
        None => path.to_string(),
    }
}

struct Draft {
    name: String,
    path: String,
    dirs: Vec<String>,
    files: HashMap<String, FileEntry>,
}

pub fn build_tree(entries: &[FileEntry]) -> Vec<TreeNode> {
    let mut drafts: HashMap<String, Draft> = HashMap::new();
    drafts.insert(
        String::new(),
        Draft {
            name: String::new(),
            path: String::new(),
            dirs: Vec::new(),
            files: HashMap::new(),
        },
    );
    for entry in entries {
        let parts: Vec<&str> = entry
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            continue;
        }
        let mut parent = String::new();
        for (index, name) in parts.iter().enumerate().take(parts.len() - 1) {
            let path = parts[..=index].join("/");
            if let Some(folder) = drafts.get_mut(&parent)
                && !folder.dirs.iter().any(|child| child == &path)
            {
                folder.dirs.push(path.clone());
            }
            drafts.entry(path.clone()).or_insert_with(|| Draft {
                name: (*name).to_string(),
                path: path.clone(),
                dirs: Vec::new(),
                files: HashMap::new(),
            });
            parent = path;
        }
        let file_name = parts[parts.len() - 1].to_string();
        if let Some(folder) = drafts.get_mut(&parent) {
            folder.files.insert(file_name, entry.clone());
        }
    }
    finish("", &drafts)
}

fn finish(path: &str, drafts: &HashMap<String, Draft>) -> Vec<TreeNode> {
    let Some(draft) = drafts.get(path) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    for child_path in &draft.dirs {
        let Some(child) = drafts.get(child_path) else {
            continue;
        };
        let children = finish(child_path, drafts);
        nodes.push(TreeNode::Dir {
            name: child.name.clone(),
            path: child.path.clone(),
            status: folder_status(&children),
            children,
        });
    }
    for file in draft.files.values() {
        nodes.push(TreeNode::File {
            name: file_name(&file.path),
            path: file.path.clone(),
            status: file.status,
        });
    }
    nodes.sort_by(cmp_node);
    nodes
}

fn file_matches(path: &str, status: FileStatus, filter: &str, changed_only: bool) -> bool {
    if changed_only && status == FileStatus::Clean {
        return false;
    }
    if !filter.is_empty() && !path.to_lowercase().contains(filter) {
        return false;
    }
    true
}

fn dir_has_match(node: &TreeNode, filter: &str, changed_only: bool) -> bool {
    if node.kind() == RowKind::File {
        return file_matches(node.path(), node.status(), filter, changed_only);
    }
    node.children()
        .iter()
        .any(|child| dir_has_match(child, filter, changed_only))
}

pub fn flatten_visible(
    nodes: &[TreeNode],
    expanded: &HashSet<String>,
    filter: &str,
    changed_only: bool,
) -> Vec<VisibleRow> {
    let filter = filter.trim().to_lowercase();
    let filtering = !filter.is_empty() || changed_only;
    let mut rows = Vec::new();
    walk(
        nodes,
        0,
        filtering,
        &filter,
        changed_only,
        expanded,
        &mut rows,
    );
    rows
}

fn walk(
    nodes: &[TreeNode],
    depth: i64,
    filtering: bool,
    filter: &str,
    changed_only: bool,
    expanded: &HashSet<String>,
    rows: &mut Vec<VisibleRow>,
) {
    for node in nodes {
        if filtering && !dir_has_match(node, filter, changed_only) {
            continue;
        }
        rows.push(VisibleRow {
            kind: node.kind(),
            name: node.name().to_string(),
            path: node.path().to_string(),
            status: node.status(),
            depth,
            has_changed: node.status() != FileStatus::Clean,
        });
        if node.kind() == RowKind::Dir {
            let open = filtering || expanded.contains(node.path());
            if open {
                walk(
                    node.children(),
                    depth + 1,
                    filtering,
                    filter,
                    changed_only,
                    expanded,
                    rows,
                );
            }
        }
    }
}

pub fn window_range(
    count: i64,
    scroll_top: f64,
    viewport_height: f64,
    row_height: f64,
) -> WindowRange {
    window_range_with_overscan(count, scroll_top, viewport_height, row_height, 8)
}

pub fn window_range_with_overscan(
    count: i64,
    scroll_top: f64,
    viewport_height: f64,
    row_height: f64,
    overscan: i64,
) -> WindowRange {
    if count <= FILE_WINDOW_THRESHOLD || row_height <= 0.0 {
        return WindowRange {
            start: 0,
            end: count,
        };
    }
    let start = ((scroll_top / row_height).floor() as i64 - overscan).max(0);
    let visible = (viewport_height / row_height).ceil() as i64 + overscan * 2;
    let end = count.min(start.saturating_add(visible));
    WindowRange { start, end }
}

pub fn parent_path(path: &str) -> Option<&str> {
    match path.rfind('/') {
        Some(index) if index > 0 => Some(&path[..index]),
        _ => None,
    }
}

pub fn count_files(entries: &[FileEntry], rows: &[VisibleRow]) -> FileCounts {
    FileCounts {
        total: entries.len(),
        shown: rows.iter().filter(|row| row.kind == RowKind::File).count(),
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    if bytes < 1_000_000 {
        let n = bytes as f64 / 1000.0;
        return if n < 10.0 {
            format!("{} KB", one_decimal(n))
        } else {
            format!("{} KB", n.round() as u64)
        };
    }
    let n = bytes as f64 / 1_000_000.0;
    if n < 10.0 {
        format!("{} MB", one_decimal(n))
    } else {
        format!("{} MB", n.round() as u64)
    }
}

fn one_decimal(n: f64) -> String {
    let scaled = (n * 10.0).round() as i64;
    let whole = scaled.div_euclid(10);
    let frac = scaled.rem_euclid(10);
    format!("{whole}.{frac}")
}

pub fn files_count_label(shown: usize, total: usize, narrowed: bool) -> String {
    let files = if total == 1 {
        "1 file".to_string()
    } else {
        format!("{total} files")
    };
    if !narrowed || shown == total {
        files
    } else {
        format!("{shown} of {total}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use legate_core::{FileEntry, FileStatus};

    use super::{
        FILE_WINDOW_THRESHOLD, TreeNode, build_tree, count_files, files_count_label,
        flatten_visible, format_bytes, parent_path, window_range, window_range_with_overscan,
    };

    fn entry(path: &str, status: FileStatus) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status,
        }
    }

    fn sample() -> Vec<FileEntry> {
        vec![
            entry("README.md", FileStatus::Clean),
            entry("src/app.ts", FileStatus::Modified),
            entry("src/New.ts", FileStatus::Added),
            entry("src/lib/util.ts", FileStatus::Clean),
            entry("notes.txt", FileStatus::Untracked),
            entry("old.txt", FileStatus::Deleted),
            entry("Zed.txt", FileStatus::Clean),
        ]
    }

    fn names(nodes: &[TreeNode]) -> Vec<&str> {
        nodes.iter().map(TreeNode::name).collect()
    }

    #[test]
    fn build_tree_puts_folders_first_then_files_alphabetical_case_insensitive() {
        let tree = build_tree(&sample());
        assert_eq!(
            names(&tree),
            vec!["src", "notes.txt", "old.txt", "README.md", "Zed.txt"]
        );
        let TreeNode::Dir { children, .. } = &tree[0] else {
            panic!("src is a folder");
        };
        assert_eq!(names(children), vec!["lib", "app.ts", "New.ts"]);
    }

    #[test]
    fn a_folder_shows_the_strongest_status_among_its_children() {
        let tree = build_tree(&[
            entry("src/a.ts", FileStatus::Untracked),
            entry("src/b.ts", FileStatus::Conflicted),
            entry("src/c.ts", FileStatus::Modified),
        ]);
        assert_eq!(tree[0].status(), FileStatus::Conflicted);
        let nested = build_tree(&[
            entry("src/lib/a.ts", FileStatus::Added),
            entry("src/b.ts", FileStatus::Clean),
        ]);
        assert_eq!(nested[0].status(), FileStatus::Added);
    }

    #[test]
    fn flatten_visible_shows_the_top_level_and_only_expanded_folders() {
        let tree = build_tree(&sample());
        let collapsed = flatten_visible(&tree, &HashSet::new(), "", false);
        assert_eq!(
            collapsed
                .iter()
                .map(|row| row.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src", "notes.txt", "old.txt", "README.md", "Zed.txt"]
        );
        let mut open_set = HashSet::new();
        open_set.insert("src".to_string());
        let open = flatten_visible(&tree, &open_set, "", false);
        assert_eq!(
            open.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
            vec![
                "src",
                "src/lib",
                "src/app.ts",
                "src/New.ts",
                "notes.txt",
                "old.txt",
                "README.md",
                "Zed.txt",
            ]
        );
        assert!(
            open.iter()
                .find(|row| row.path == "src")
                .is_some_and(|row| row.has_changed)
        );
        assert!(
            open.iter()
                .find(|row| row.path == "src/lib")
                .is_some_and(|row| !row.has_changed)
        );
    }

    #[test]
    fn filter_keeps_matching_files_and_auto_expands_their_parents() {
        let tree = build_tree(&sample());
        let rows = flatten_visible(&tree, &HashSet::new(), "APP", false);
        assert_eq!(
            rows.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
            vec!["src", "src/app.ts"]
        );
    }

    #[test]
    fn changed_only_hides_clean_files_and_empty_folders() {
        let tree = build_tree(&sample());
        let rows = flatten_visible(&tree, &HashSet::new(), "", true);
        assert_eq!(
            rows.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
            vec!["src", "src/app.ts", "src/New.ts", "notes.txt", "old.txt"]
        );
    }

    #[test]
    fn count_and_label() {
        let tree = build_tree(&sample());
        let entries = sample();
        let mut open = HashSet::new();
        open.insert("src".to_string());
        open.insert("src/lib".to_string());
        let all = flatten_visible(&tree, &open, "", false);
        assert_eq!(count_files(&entries, &all).shown, 7);
        assert_eq!(count_files(&entries, &all).total, 7);
        let filtered = flatten_visible(&tree, &HashSet::new(), "ts", false);
        assert_eq!(count_files(&entries, &filtered).shown, 3);
        assert_eq!(files_count_label(7, 7, false), "7 files");
        assert_eq!(files_count_label(1, 1, false), "1 file");
        assert_eq!(files_count_label(4, 7, true), "4 of 7");
    }

    #[test]
    fn window_range_renders_everything_under_the_threshold_and_a_slice_above_it() {
        assert_eq!(window_range(10, 0.0, 200.0, 24.0).start, 0);
        assert_eq!(window_range(10, 0.0, 200.0, 24.0).end, 10);
        assert_eq!(
            window_range(FILE_WINDOW_THRESHOLD, 0.0, 200.0, 24.0).end,
            FILE_WINDOW_THRESHOLD
        );
        let ranged = window_range_with_overscan(2000, 240.0, 120.0, 24.0, 2);
        assert_eq!(ranged.start, 8);
        assert_eq!(ranged.end, 8 + 5 + 4);
    }

    #[test]
    fn parent_path_and_format_bytes() {
        assert_eq!(parent_path("src/lib/util.ts"), Some("src/lib"));
        assert_eq!(parent_path("README.md"), None);
        assert_eq!(format_bytes(12), "12 B");
        assert_eq!(format_bytes(1200), "1.2 KB");
        assert_eq!(format_bytes(12_000), "12 KB");
        assert_eq!(format_bytes(2_500_000), "2.5 MB");
    }
}
