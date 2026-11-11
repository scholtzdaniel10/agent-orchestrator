//! Edited-file totals and the latest step title. Port of `job-steps.ts`.
//!
//! Diff classification lives here too so a job can show its patch before the
//! changes slice finishes `diff-lines.ts`. The rules match that file.

use legate_core::{JobRecord, JobStep, StepTool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedFile {
    pub path: String,
    pub added: i64,
    pub removed: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditSummary {
    pub files: usize,
    pub added: i64,
    pub removed: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    File,
    Hunk,
    Add,
    Del,
    Context,
    Meta,
}

impl DiffKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Hunk => "hunk",
            Self::Add => "add",
            Self::Del => "del",
            Self::Context => "context",
            Self::Meta => "meta",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: DiffKind,
    pub text: String,
}

pub fn latest_step_title(job: &JobRecord) -> Option<String> {
    job.steps
        .as_ref()
        .and_then(|steps| steps.last())
        .map(|step| step.title.clone())
}

pub fn edited_files_summary(job: &JobRecord) -> Option<EditSummary> {
    let files = edited_files(job.steps.as_deref().unwrap_or(&[]));
    if files.is_empty() {
        return None;
    }
    let mut added = 0;
    let mut removed = 0;
    for file in &files {
        added += file.added;
        removed += file.removed;
    }
    Some(EditSummary {
        files: files.len(),
        added,
        removed,
    })
}

pub fn edited_files(steps: &[JobStep]) -> Vec<EditedFile> {
    let mut files: Vec<EditedFile> = Vec::new();
    for step in steps {
        if step.tool != StepTool::Edit {
            continue;
        }
        let Some(edit) = &step.edit else {
            continue;
        };
        if let Some(file) = files.iter_mut().find(|file| file.path == edit.path) {
            file.added += edit.added;
            file.removed += edit.removed;
        } else {
            files.push(EditedFile {
                path: edit.path.clone(),
                added: edit.added,
                removed: edit.removed,
            });
        }
    }
    files
}

pub fn parse_diff(text: &str) -> Vec<DiffRow> {
    if text.is_empty() {
        return Vec::new();
    }
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(|line| DiffRow {
            kind: classify(line),
            text: line.to_string(),
        })
        .collect()
}

fn classify(line: &str) -> DiffKind {
    if line.starts_with("… diff cut") {
        return DiffKind::Meta;
    }
    if line.starts_with("diff --git") {
        return DiffKind::File;
    }
    if line.starts_with("@@") {
        return DiffKind::Hunk;
    }
    const META_PREFIXES: [&str; 8] = [
        "+++",
        "---",
        "index ",
        "new file",
        "deleted file",
        "similarity",
        "rename",
        "Binary files",
    ];
    if META_PREFIXES.iter().any(|prefix| line.starts_with(prefix)) {
        return DiffKind::Meta;
    }
    if line.starts_with('+') {
        return DiffKind::Add;
    }
    if line.starts_with('-') {
        return DiffKind::Del;
    }
    DiffKind::Context
}

/// Lines of `text` that belong to `file_path`, including its `diff --git` header.
pub fn diff_for_file(text: &str, file_path: &str) -> Vec<DiffRow> {
    let mut out = Vec::new();
    let mut keep = false;
    let wanted = file_path.replace('\\', "/");
    for line in parse_diff(text) {
        if line.kind == DiffKind::File {
            keep = git_header_matches(&line.text, &wanted);
        }
        if keep {
            out.push(line);
        }
    }
    out
}

fn git_header_matches(header: &str, file_path: &str) -> bool {
    let Some(rest) = header.strip_prefix("diff --git a/") else {
        return header.contains(file_path);
    };
    let Some(split) = rest.rfind(" b/") else {
        return header.contains(file_path);
    };
    let left = &rest[..split];
    let right = &rest[split + 3..];
    path_matches(file_path, left) || path_matches(file_path, right)
}

fn path_matches(file_path: &str, git_path: &str) -> bool {
    let left = file_path.replace('\\', "/");
    let right = git_path.replace('\\', "/");
    left == right || left.ends_with(&format!("/{right}")) || right.ends_with(&format!("/{left}"))
}

#[cfg(test)]
mod tests {
    use super::{DiffKind, edited_files, edited_files_summary, latest_step_title, parse_diff};
    use legate_core::{
        FileEdit, JobRecord, JobStatus, JobStep, JobStepStatus, JobType, ProviderId, StepTool,
    };
    use std::collections::BTreeMap;

    fn job() -> JobRecord {
        JobRecord {
            id: "j".into(),
            job_type: JobType::Refactor,
            prompt: "prompt".into(),
            provider: Some(ProviderId::Claude),
            status: JobStatus::Running,
            output: String::new(),
            failed_over: Vec::new(),
            steps: None,
            steps_dropped: None,
            model: None,
            reason: None,
            edit: None,
            access: None,
            change: None,
            group: None,
            lead_message: None,
            routine: None,
            title: None,
            session_id: None,
            goal: None,
            base: None,
            error: None,
            owner_id: None,
            heartbeat_at: None,
            cancel_requested: None,
            extra: BTreeMap::new(),
        }
    }

    fn edit(path: &str, added: i64, removed: i64) -> JobStep {
        JobStep {
            id: path.into(),
            tool: StepTool::Edit,
            title: format!("Edited {path}"),
            detail: None,
            status: JobStepStatus::Done,
            started_at: 0,
            ended_at: Some(1),
            edit: Some(FileEdit {
                path: path.into(),
                added,
                removed,
            }),
        }
    }

    #[test]
    fn latest_title_is_the_last_step() {
        let mut record = job();
        assert_eq!(latest_step_title(&record), None);
        record.steps = Some(vec![edit("a.rs", 1, 0), {
            let mut step = edit("b.rs", 2, 1);
            step.title = "Edited b.rs".into();
            step
        }]);
        assert_eq!(latest_step_title(&record).as_deref(), Some("Edited b.rs"));
    }

    #[test]
    fn edits_sum_by_path_in_first_seen_order() {
        let steps = vec![
            edit("src/a.rs", 1, 0),
            edit("src/b.rs", 2, 3),
            edit("src/a.rs", 4, 1),
        ];
        let files = edited_files(&steps);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].added, 5);
        assert_eq!(files[0].removed, 1);
        assert_eq!(files[1].path, "src/b.rs");
        let mut record = job();
        record.steps = Some(steps);
        let summary = edited_files_summary(&record).unwrap();
        assert_eq!(summary.files, 2);
        assert_eq!(summary.added, 7);
        assert_eq!(summary.removed, 4);
    }

    #[test]
    fn diff_headers_that_start_with_plus_or_minus_stay_meta() {
        let rows =
            parse_diff("diff --git a/n.txt b/n.txt\n--- a/n.txt\n+++ b/n.txt\n+added\n-gone\n");
        assert_eq!(rows[0].kind, DiffKind::File);
        assert_eq!(rows[1].kind, DiffKind::Meta);
        assert_eq!(rows[2].kind, DiffKind::Meta);
        assert_eq!(rows[3].kind, DiffKind::Add);
        assert_eq!(rows[4].kind, DiffKind::Del);
    }
}
