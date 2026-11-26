//! Worktree names on cards and terminal tabs. Port of `worktree-label.ts`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedChange {
    pub id: String,
    pub name: Option<String>,
}

pub fn worktree_name_from_task(task: &str) -> String {
    let text = task.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 32 {
        return text;
    }
    if chars.get(32).copied() == Some(' ') {
        return chars.iter().take(32).collect();
    }
    let slice: String = chars.iter().take(32).collect();
    match slice.rfind(' ') {
        Some(break_at) if break_at > 0 => slice[..break_at].to_string(),
        _ => slice,
    }
}

pub fn worktree_label(change: &NamedChange) -> String {
    match change.name.as_deref() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => change.id.clone(),
    }
}

/// A worktree's branch only when it says something its name does not: Legate's own
/// `orch/<id>` is just the id again, so it is left to tooltips and the place menu.
pub fn telling_branch<'a>(branch: &'a str, id: &str) -> Option<&'a str> {
    let own = branch.strip_prefix("orch/").is_some_and(|rest| rest == id);
    (!own && !branch.is_empty()).then_some(branch)
}

/// The words for a branch the project folder is on: a worktree's name when it is one of
/// Legate's `orch/<id>` branches, the branch itself otherwise.
pub fn branch_place_label(branch: &str, changes: &[NamedChange]) -> String {
    branch
        .strip_prefix("orch/")
        .and_then(|id| changes.iter().find(|change| change.id == id))
        .map(worktree_label)
        .unwrap_or_else(|| branch.to_string())
}

pub fn terminal_display_title(
    title: &str,
    change: Option<&str>,
    changes: &[NamedChange],
) -> String {
    let Some(change_id) = change else {
        return title.to_string();
    };
    let found = changes.iter().find(|item| item.id == change_id);
    let label = match found {
        Some(item) => worktree_label(item),
        None => change_id.to_string(),
    };
    let suffix = format!(" · {change_id}");
    if title.ends_with(&suffix) {
        let stem: String = title
            .chars()
            .take(title.chars().count() - change_id.chars().count())
            .collect();
        format!("{stem}{label}")
    } else {
        title.to_string()
    }
}

pub fn terminals_in_sidebar_order<T, F>(
    terminals: &[T],
    changes: &[NamedChange],
    change_of: F,
) -> Vec<T>
where
    T: Clone,
    F: Fn(&T) -> Option<&str>,
{
    let mut indexed: Vec<(usize, T)> = terminals.iter().cloned().enumerate().collect();
    indexed.sort_by(|(index_a, a), (index_b, b)| {
        let rank = |change: Option<&str>| match change {
            None => 0,
            Some(id) => changes
                .iter()
                .position(|item| item.id == id)
                .map(|index| index + 1)
                .unwrap_or(usize::MAX),
        };
        rank(change_of(a))
            .cmp(&rank(change_of(b)))
            .then(index_a.cmp(index_b))
    });
    indexed.into_iter().map(|(_, info)| info).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        NamedChange, terminal_display_title, terminals_in_sidebar_order, worktree_label,
        worktree_name_from_task,
    };

    #[test]
    fn own_orch_branches_are_not_repeated_under_the_name() {
        assert_eq!(super::telling_branch("orch/3fa9c1d2", "3fa9c1d2"), None);
        assert_eq!(
            super::telling_branch("orch/pr-fix-login-1a2b", "3fa9c1d2"),
            Some("orch/pr-fix-login-1a2b")
        );
        assert_eq!(
            super::telling_branch("feature/login", "3fa9c1d2"),
            Some("feature/login")
        );
        assert_eq!(super::telling_branch("", "3fa9c1d2"), None);
    }

    #[test]
    fn a_project_on_a_worktree_branch_reads_as_that_worktree() {
        let changes = vec![
            NamedChange {
                id: "3fa9c1d2".into(),
                name: Some("Fix the login form".into()),
            },
            NamedChange {
                id: "77aa0011".into(),
                name: None,
            },
        ];
        assert_eq!(
            super::branch_place_label("orch/3fa9c1d2", &changes),
            "Fix the login form"
        );
        assert_eq!(
            super::branch_place_label("orch/77aa0011", &changes),
            "77aa0011"
        );
        assert_eq!(
            super::branch_place_label("orch/deadbeef", &changes),
            "orch/deadbeef"
        );
        assert_eq!(super::branch_place_label("main", &changes), "main");
    }

    #[test]
    fn names_take_the_first_32_characters_at_a_word() {
        assert_eq!(worktree_name_from_task("fix the login"), "fix the login");
        assert_eq!(
            worktree_name_from_task("  Fix the login form validation on the settings page  "),
            "Fix the login form validation on"
        );
        assert_eq!(
            worktree_name_from_task("one two three four five six seven eight"),
            "one two three four five six"
        );
        assert_eq!(
            worktree_name_from_task("abcdefghijklmnopqrstuvwxyz0123456789"),
            "abcdefghijklmnopqrstuvwxyz012345"
        );
        assert_eq!(
            worktree_name_from_task("line one\nline two"),
            "line one line two"
        );
    }

    #[test]
    fn label_uses_the_name_when_present() {
        assert_eq!(
            worktree_label(&NamedChange {
                id: "3fa9c1d2".into(),
                name: None
            }),
            "3fa9c1d2"
        );
        assert_eq!(
            worktree_label(&NamedChange {
                id: "3fa9c1d2".into(),
                name: Some(String::new())
            }),
            "3fa9c1d2"
        );
        assert_eq!(
            worktree_label(&NamedChange {
                id: "3fa9c1d2".into(),
                name: Some("my-feature".into())
            }),
            "my-feature"
        );
    }

    #[test]
    fn display_title_swaps_the_id_suffix() {
        let named = vec![NamedChange {
            id: "3fa9c1d2".into(),
            name: Some("my-feature".into()),
        }];
        assert_eq!(
            terminal_display_title("claude 2 · 3fa9c1d2", Some("3fa9c1d2"), &named),
            "claude 2 · my-feature"
        );
        assert_eq!(
            terminal_display_title("claude 2 · 3fa9c1d2", Some("3fa9c1d2"), &[]),
            "claude 2 · 3fa9c1d2"
        );
        assert_eq!(terminal_display_title("claude 1", None, &named), "claude 1");
    }

    #[test]
    fn sidebar_order_puts_the_project_folder_first() {
        let changes = vec![
            NamedChange {
                id: "wt-a".into(),
                name: None,
            },
            NamedChange {
                id: "wt-b".into(),
                name: None,
            },
        ];
        let terminals = [
            ("1", Some("wt-b")),
            ("2", None),
            ("3", Some("wt-a")),
            ("4", Some("wt-b")),
            ("5", Some("gone")),
        ];
        let ordered = terminals_in_sidebar_order(&terminals, &changes, |item| item.1);
        let ids: Vec<&str> = ordered.iter().map(|item| item.0).collect();
        assert_eq!(ids, ["2", "3", "1", "4", "5"]);
    }
}
