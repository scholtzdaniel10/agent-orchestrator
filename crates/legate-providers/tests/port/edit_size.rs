use legate_providers::{
    LineChange, cap_detail, edit_size, first_line, line_count, relative_to_cwd, reported_edit_size,
    sum_edit_size, write_size,
};
use serde_json::json;

fn lines(added: i64, removed: i64) -> LineChange {
    LineChange { added, removed }
}

#[test]
fn line_count_treats_a_trailing_newline_as_the_last_line_ending() {
    assert_eq!(line_count(""), 0);
    assert_eq!(line_count("hello"), 1);
    assert_eq!(line_count("hello\n"), 1);
    assert_eq!(line_count("hello\nworld"), 2);
    assert_eq!(line_count("hello\nworld\n"), 2);
    assert_eq!(line_count("\n"), 1);
}

#[test]
fn edit_size_is_new_lines_vs_old_lines_write_size_only_adds() {
    assert_eq!(edit_size("a\nb\n", "a\nb\nc\n"), lines(3, 2));
    assert_eq!(edit_size("", "one\n"), lines(1, 0));
    assert_eq!(write_size("alpha\nbeta\n"), lines(2, 0));
    assert_eq!(write_size(""), lines(0, 0));
    assert_eq!(
        sum_edit_size(&[edit_size("a", "aa"), write_size("x\ny")]),
        lines(3, 1)
    );
}

#[test]
fn reported_edit_size_reads_cursor_style_count_fields() {
    assert_eq!(
        reported_edit_size(&json!({"addedLines": 4, "removedLines": 1})),
        Some(lines(4, 1))
    );
    assert_eq!(
        reported_edit_size(&json!({"success": {"insertions": 2, "deletions": 3}})),
        Some(lines(2, 3))
    );
    assert_eq!(reported_edit_size(&json!({"added": 1})), None);
    assert_eq!(reported_edit_size(&serde_json::Value::Null), None);
}

#[test]
fn relative_to_cwd_uses_forward_slashes_and_only_strips_a_matching_prefix() {
    assert_eq!(
        relative_to_cwd("/work/example/src/app.ts", Some("/work/example")),
        "src/app.ts"
    );
    assert_eq!(relative_to_cwd("/work/example", Some("/work/example")), ".");
    assert_eq!(
        relative_to_cwd("/other/app.ts", Some("/work/example")),
        "/other/app.ts"
    );
    assert_eq!(
        relative_to_cwd("C:\\work\\example\\a.ts", Some("C:\\work\\example")),
        "a.ts"
    );
    assert_eq!(
        relative_to_cwd("/work/example/a.ts", None),
        "/work/example/a.ts"
    );
}

#[test]
fn cap_detail_keeps_the_tail_first_line_cuts_to_120() {
    assert_eq!(cap_detail("short"), "short");
    let long = "x".repeat(4001);
    assert_eq!(cap_detail(&long).chars().count(), 4000);
    assert!(cap_detail(&long).starts_with('x'));
    assert_eq!(first_line("hello\nworld"), "hello");
    assert_eq!(first_line(&"a".repeat(121)).chars().count(), 120);
}
