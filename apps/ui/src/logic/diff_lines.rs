//! Coloured lines of a unified diff. Port of `diff-lines.ts`.

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
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
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

const CUT_PREFIX: &str = "\u{2026} diff cut";

fn classify(line: &str) -> DiffKind {
    if line.starts_with(CUT_PREFIX) {
        return DiffKind::Meta;
    }
    if line.starts_with("diff --git") {
        return DiffKind::File;
    }
    if line.starts_with("@@") {
        return DiffKind::Hunk;
    }
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

/// Split a unified diff into coloured lines. Headers that start with + or - stay meta.
pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    if text.is_empty() {
        return Vec::new();
    }
    split_newlines(text)
        .into_iter()
        .map(|line| DiffLine {
            kind: classify(line),
            text: line.to_string(),
        })
        .collect()
}

/// `String.split(/\r?\n/)`: a trailing newline yields a final empty line. A lone CR is not a break.
fn split_newlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            lines.push(&text[start..end]);
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    lines.push(&text[start..]);
    lines
}

#[cfg(test)]
mod tests {
    use super::{DiffKind, parse_diff};

    fn kinds(text: &str) -> Vec<DiffKind> {
        parse_diff(text).into_iter().map(|line| line.kind).collect()
    }

    #[test]
    fn a_modified_file_classifies_headers_hunks_and_changed_lines() {
        let text = [
            "diff --git a/notes.txt b/notes.txt",
            "index 1111111..2222222 100644",
            "--- a/notes.txt",
            "+++ b/notes.txt",
            "@@ -1,2 +1,3 @@",
            " first",
            "-second",
            "+second changed",
            "+third",
        ]
        .join("\n");
        let lines = parse_diff(&text);
        let expected = [
            (DiffKind::File, "diff --git a/notes.txt b/notes.txt"),
            (DiffKind::Meta, "index 1111111..2222222 100644"),
            (DiffKind::Meta, "--- a/notes.txt"),
            (DiffKind::Meta, "+++ b/notes.txt"),
            (DiffKind::Hunk, "@@ -1,2 +1,3 @@"),
            (DiffKind::Context, " first"),
            (DiffKind::Del, "-second"),
            (DiffKind::Add, "+second changed"),
            (DiffKind::Add, "+third"),
        ];
        assert_eq!(lines.len(), expected.len());
        for (line, (kind, text)) in lines.iter().zip(expected) {
            assert_eq!(line.kind, kind);
            assert_eq!(line.text, text);
        }
    }

    #[test]
    fn a_new_file_keeps_the_headers_as_meta_and_the_body_as_additions() {
        let text = [
            "diff --git a/fresh.txt b/fresh.txt",
            "new file mode 100644",
            "index 0000000..abc1234",
            "--- /dev/null",
            "+++ b/fresh.txt",
            "@@ -0,0 +1 @@",
            "+hello",
        ]
        .join("\n");
        assert_eq!(
            kinds(&text),
            vec![
                DiffKind::File,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Hunk,
                DiffKind::Add,
            ]
        );
    }

    #[test]
    fn a_deleted_file_keeps_the_headers_as_meta_and_the_body_as_deletions() {
        let text = [
            "diff --git a/old.txt b/old.txt",
            "deleted file mode 100644",
            "index abc1234..0000000",
            "--- a/old.txt",
            "+++ /dev/null",
            "@@ -1 +0,0 @@",
            "-goodbye",
        ]
        .join("\n");
        assert_eq!(
            kinds(&text),
            vec![
                DiffKind::File,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Hunk,
                DiffKind::Del,
            ]
        );
    }

    #[test]
    fn a_rename_is_file_headers_plus_meta_with_no_add_or_del() {
        let text = [
            "diff --git a/old.txt b/new.txt",
            "similarity index 100%",
            "rename from old.txt",
            "rename to new.txt",
        ]
        .join("\n");
        assert_eq!(
            kinds(&text),
            vec![
                DiffKind::File,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Meta
            ]
        );
    }

    #[test]
    fn a_binary_file_is_meta_through_the_binary_marker() {
        let text = [
            "diff --git a/pic.bin b/pic.bin",
            "new file mode 100644",
            "index 0000000..abcdef0",
            "Binary files /dev/null and b/pic.bin differ",
        ]
        .join("\n");
        assert_eq!(
            kinds(&text),
            vec![
                DiffKind::File,
                DiffKind::Meta,
                DiffKind::Meta,
                DiffKind::Meta
            ]
        );
    }

    #[test]
    fn the_cut_marker_is_meta() {
        let text = [
            "diff --git a/a.txt b/a.txt",
            "+kept",
            "\u{2026} diff cut at 400000 characters",
        ]
        .join("\n");
        let lines = parse_diff(&text);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, DiffKind::File);
        assert_eq!(lines[0].text, "diff --git a/a.txt b/a.txt");
        assert_eq!(lines[1].kind, DiffKind::Add);
        assert_eq!(lines[1].text, "+kept");
        assert_eq!(lines[2].kind, DiffKind::Meta);
        assert_eq!(lines[2].text, "\u{2026} diff cut at 400000 characters");
        let only = parse_diff("\u{2026} diff cut \u{2026}");
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].kind, DiffKind::Meta);
        assert_eq!(only[0].text, "\u{2026} diff cut \u{2026}");
    }

    #[test]
    fn an_empty_string_yields_no_lines() {
        assert!(parse_diff("").is_empty());
    }

    #[test]
    fn plus_and_minus_headers_are_meta_not_add_or_del() {
        let lines = parse_diff("--- a/notes.txt\n+++ b/notes.txt");
        assert_eq!(
            lines.iter().map(|line| line.kind).collect::<Vec<_>>(),
            vec![DiffKind::Meta, DiffKind::Meta]
        );
        assert!(
            lines
                .iter()
                .all(|line| line.kind != DiffKind::Add && line.kind != DiffKind::Del)
        );
    }
}
