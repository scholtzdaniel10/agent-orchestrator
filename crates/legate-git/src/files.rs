//! Project file list, read, and diff against HEAD.

use std::path::{Path, PathBuf};

use legate_core::{FileContent, FileEntry, FileList, FileStatus};

use crate::cmd::{CmdOut, CmdRun};
use crate::error::GitError;
use crate::gitpath::{
    canonicalize_path, git_path, is_inside, normalize_abs, read_until_nul, slash,
};

pub const OUTSIDE_PROJECT: &str = "That file is outside the project.";

const LIST_CAP: usize = 20_000;
const TEXT_CAP: u64 = 1_000_000;
const BINARY_PROBE: usize = 8000;

#[derive(Debug, Clone)]
pub struct FileStat {
    pub is_file: bool,
    pub is_dir: bool,
    pub size: u64,
}

pub trait FileSys: Send + Sync {
    fn realpath(&self, target: &Path) -> Option<PathBuf>;
    fn read_file(&self, target: &Path) -> std::io::Result<Vec<u8>>;
    fn stat(&self, target: &Path) -> std::io::Result<FileStat>;
    fn readdir(&self, dir: &Path) -> std::io::Result<Vec<String>>;
}

#[derive(Debug, Clone, Copy)]
pub struct OsFs;

impl FileSys for OsFs {
    fn realpath(&self, target: &Path) -> Option<PathBuf> {
        canonicalize_path(target)
    }

    fn read_file(&self, target: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(target)
    }

    fn stat(&self, target: &Path) -> std::io::Result<FileStat> {
        let meta = std::fs::metadata(target)?;
        Ok(FileStat {
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            size: meta.len(),
        })
    }

    fn readdir(&self, dir: &Path) -> std::io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }
}

pub struct Files<R, F> {
    run: R,
    fs: F,
}

impl Files<crate::cmd::ProcessRun, OsFs> {
    pub fn new() -> Self {
        Self {
            run: crate::cmd::ProcessRun::command(),
            fs: OsFs,
        }
    }
}

impl Default for Files<crate::cmd::ProcessRun, OsFs> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: CmdRun, F: FileSys> Files<R, F> {
    pub fn with(run: R, fs: F) -> Self {
        Self { run, fs }
    }

    pub async fn list(&self, root: &Path) -> FileList {
        let inside = self
            .run
            .run(
                "git".into(),
                vec!["rev-parse".into(), "--is-inside-work-tree".into()],
                Some(root.to_path_buf()),
            )
            .await;
        if inside.code != 0 || inside.stdout.trim() != "true" {
            return self.walk(root);
        }
        let listed = self.run.run(
            "git".into(),
            vec![
                "ls-files".into(),
                "--cached".into(),
                "--others".into(),
                "--exclude-standard".into(),
                "-z".into(),
            ],
            Some(root.to_path_buf()),
        );
        let status = self.run.run(
            "git".into(),
            vec![
                "status".into(),
                "--porcelain=v2".into(),
                "-z".into(),
                "--untracked-files=all".into(),
            ],
            Some(root.to_path_buf()),
        );
        let (listed, status) = tokio::join!(listed, status);
        let mut by_path = std::collections::BTreeMap::<String, FileStatus>::new();
        if listed.code == 0 {
            for path in split_z_paths(listed.stdout_z()) {
                let rel = slash(&path);
                if rel.is_empty() {
                    continue;
                }
                by_path.insert(rel, FileStatus::Clean);
            }
        }
        if status.code == 0 {
            for (path, file_status) in parse_status_v2(status.stdout_z()) {
                by_path.insert(path, file_status);
            }
        }
        finish_list(
            by_path
                .into_iter()
                .map(|(path, status)| FileEntry { path, status })
                .collect(),
            false,
        )
    }

    pub async fn read(&self, root: &Path, path: &str) -> Result<FileContent, GitError> {
        let abs = self.resolve(root, path)?;
        let info = match self.fs.stat(&abs) {
            Ok(info) => info,
            Err(_) => return Ok(FileContent::Missing),
        };
        if !info.is_file {
            return Ok(FileContent::Missing);
        }
        let bytes = i64::try_from(info.size).unwrap_or(i64::MAX);
        if info.size > TEXT_CAP {
            return Ok(FileContent::TooLarge { bytes });
        }
        let buf = self.fs.read_file(&abs)?;
        let bytes = i64::try_from(buf.len()).unwrap_or(i64::MAX);
        let probe = buf.len().min(BINARY_PROBE);
        if buf[..probe].contains(&0) {
            return Ok(FileContent::Binary { bytes });
        }
        Ok(FileContent::Text {
            text: String::from_utf8_lossy(&buf).into_owned(),
            bytes,
        })
    }

    pub async fn diff(&self, root: &Path, path: &str) -> Result<String, GitError> {
        let rel = slash(path);
        self.resolve(root, &rel)?;
        let tracked = self
            .run
            .run(
                "git".into(),
                vec![
                    "ls-files".into(),
                    "--error-unmatch".into(),
                    "--".into(),
                    rel.clone(),
                ],
                Some(root.to_path_buf()),
            )
            .await;
        if tracked.code != 0 {
            let added = self
                .run
                .run(
                    "git".into(),
                    vec![
                        "diff".into(),
                        "--no-index".into(),
                        "--".into(),
                        "/dev/null".into(),
                        rel,
                    ],
                    Some(root.to_path_buf()),
                )
                .await;
            return Ok(diff_stdout(added));
        }
        let result = self
            .run
            .run(
                "git".into(),
                vec!["diff".into(), "HEAD".into(), "--".into(), rel],
                Some(root.to_path_buf()),
            )
            .await;
        Ok(diff_stdout(result))
    }

    pub fn resolve(&self, root: &Path, path: &str) -> Result<PathBuf, GitError> {
        assert_safe_relative(path)?;
        let root_real = self
            .fs
            .realpath(root)
            .unwrap_or_else(|| normalize_abs(root));
        let target = normalize_abs(&root_real.join(path));
        if let Some(real_target) = self.fs.realpath(&target) {
            return if is_inside(&root_real, &real_target, true) {
                Ok(real_target)
            } else {
                Err(outside())
            };
        }
        let mut current = target.clone();
        while let Some(parent) = current.parent() {
            if parent.as_os_str().is_empty() || parent == current {
                break;
            }
            match self.fs.realpath(parent) {
                Some(real_parent) if !is_inside(&root_real, &real_parent, true) => {
                    return Err(outside());
                }
                Some(_) => break,
                None => {}
            }
            current = parent.to_path_buf();
        }
        if !is_inside(&root_real, &normalize_abs(&target), true) {
            return Err(outside());
        }
        Ok(target)
    }

    fn walk(&self, root: &Path) -> FileList {
        let mut entries = Vec::new();
        let mut truncated = false;
        collect(&self.fs, root, "", &mut entries, &mut truncated);
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        finish_list(entries, truncated)
    }
}

fn finish_list(mut entries: Vec<FileEntry>, truncated: bool) -> FileList {
    if entries.len() > LIST_CAP {
        entries.truncate(LIST_CAP);
        return FileList {
            entries,
            truncated: true,
        };
    }
    FileList { entries, truncated }
}

fn diff_stdout(result: CmdOut) -> String {
    if result.code == 0 || result.code == 1 {
        result.stdout
    } else {
        String::new()
    }
}

fn outside() -> GitError {
    GitError::new(OUTSIDE_PROJECT)
}

fn assert_safe_relative(path: &str) -> Result<(), GitError> {
    if path.is_empty() || path.contains('\0') {
        return Err(outside());
    }
    if std::path::Path::new(path).is_absolute()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.starts_with("//")
        || path.starts_with(r"\\")
    {
        return Err(outside());
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(outside());
    }
    for part in path.split(['/', '\\']) {
        if part == ".." {
            return Err(outside());
        }
    }
    Ok(())
}

fn split_z_paths(bytes: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let (field, next) = read_until_nul(bytes, index);
        index = next;
        if field.is_empty() {
            continue;
        }
        paths.push(git_path(field));
    }
    paths
}

pub(crate) fn parse_status_v2(bytes: &[u8]) -> Vec<(String, FileStatus)> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0 {
            index += 1;
            continue;
        }
        if bytes[index] == b'#' {
            let (_line, next) = read_until_nul(bytes, index);
            index = next;
            continue;
        }
        let kind = bytes[index];
        let (record, next) = read_until_nul(bytes, index);
        index = next;
        let orig = if kind == b'2' {
            let (_orig, next) = read_until_nul(bytes, index);
            index = next;
            true
        } else {
            false
        };
        let _ = orig;
        if kind == b'!' {
            continue;
        }
        let Some(path_bytes) = path_after(kind, record) else {
            continue;
        };
        let path = slash(&git_path(path_bytes));
        if path.is_empty() {
            continue;
        }
        out.push((path, status_of(kind, record)));
    }
    out
}

fn path_after(kind: u8, record: &[u8]) -> Option<&[u8]> {
    let fields = match kind {
        b'?' | b'!' => 1,
        b'1' => 8,
        b'2' => 9,
        b'u' => 10,
        _ => return None,
    };
    after_fields(record, fields)
}

fn after_fields(record: &[u8], n: usize) -> Option<&[u8]> {
    let mut rest = record;
    for index in 0..n {
        let space = rest.iter().position(|&byte| byte == b' ')?;
        if index + 1 == n {
            return Some(&rest[space + 1..]);
        }
        rest = &rest[space + 1..];
    }
    None
}

fn field(record: &[u8], index: usize) -> Option<&[u8]> {
    let mut rest = record;
    for current in 0..=index {
        let end = rest
            .iter()
            .position(|&byte| byte == b' ')
            .unwrap_or(rest.len());
        if current == index {
            return Some(&rest[..end]);
        }
        if end == rest.len() {
            return None;
        }
        rest = &rest[end + 1..];
    }
    None
}

fn status_of(kind: u8, record: &[u8]) -> FileStatus {
    let xy = if kind == b'?' {
        *b"??"
    } else {
        let token = field(record, 1).unwrap_or(b"");
        [
            token.first().copied().unwrap_or(b' '),
            token.get(1).copied().unwrap_or(b' '),
        ]
    };
    let x = xy[0];
    let y = xy[1];
    if x == b'?' && y == b'?' {
        FileStatus::Untracked
    } else if x == b'U' || y == b'U' || xy == *b"AA" || xy == *b"DD" {
        FileStatus::Conflicted
    } else if x == b'R' || y == b'R' || x == b'C' || y == b'C' {
        FileStatus::Renamed
    } else if x == b'A' || y == b'A' {
        FileStatus::Added
    } else if x == b'D' || y == b'D' {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    }
}

fn collect<F: FileSys>(
    fs: &F,
    root: &Path,
    prefix: &str,
    entries: &mut Vec<FileEntry>,
    truncated: &mut bool,
) {
    if entries.len() >= LIST_CAP {
        *truncated = true;
        return;
    }
    let abs = if prefix.is_empty() {
        root.to_path_buf()
    } else {
        root.join(prefix)
    };
    let Ok(names) = fs.readdir(&abs) else {
        return;
    };
    for name in names {
        if entries.len() >= LIST_CAP {
            *truncated = true;
            return;
        }
        if prefix.is_empty() {
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
        } else if name == "node_modules" || name == ".git" {
            continue;
        }
        let rel = if prefix.is_empty() {
            slash(&name)
        } else {
            slash(&format!("{prefix}/{name}"))
        };
        let full = root.join(&rel);
        let Ok(info) = fs.stat(&full) else {
            continue;
        };
        if info.is_dir {
            collect(fs, root, &rel, entries, truncated);
            if *truncated {
                return;
            }
        } else if info.is_file {
            entries.push(FileEntry {
                path: rel,
                status: FileStatus::Clean,
            });
        }
    }
}
