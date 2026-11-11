use legate_core::{ChangeSet, ChangedFile, SetupState};

use crate::changes::{ListedName, merge_listed_changes};

fn change(id: &str, files: i64) -> ChangeSet {
    ChangeSet {
        id: id.to_string(),
        branch: format!("orch/{id}"),
        path: format!("/wt/{id}"),
        files: if files == 0 {
            Vec::new()
        } else {
            vec![ChangedFile {
                path: "a.ts".to_string(),
                insertions: Some(2),
                deletions: Some(1),
            }]
        },
        insertions: if files == 0 { 0 } else { 2 },
        deletions: if files == 0 { 0 } else { 1 },
        name: None,
        setup: None,
    }
}

#[test]
fn merge_keeps_empty_worktrees_names_setup_and_newest_first() {
    let merged = merge_listed_changes(
        &[change("bbbbbbbb", 1)],
        &["bbbbbbbb".to_string(), "aaaaaaaa".to_string()],
        &[
            ListedName {
                id: "aaaaaaaa".to_string(),
                name: "login".to_string(),
                created_at: 20,
            },
            ListedName {
                id: "bbbbbbbb".to_string(),
                name: String::new(),
                created_at: 5,
            },
        ],
        |id| format!("/root/{id}"),
        |id| {
            if id == "aaaaaaaa" {
                Some(SetupState::Running)
            } else {
                None
            }
        },
    );
    assert_eq!(
        merged
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["aaaaaaaa", "bbbbbbbb"]
    );
    assert_eq!(merged[0].name.as_deref(), Some("login"));
    assert_eq!(merged[0].path, "/root/aaaaaaaa");
    assert!(merged[0].files.is_empty());
    assert_eq!(merged[0].setup, Some(SetupState::Running));
    assert_eq!(merged[1].name, None);
    assert_eq!(merged[1].files.len(), 1);
}
