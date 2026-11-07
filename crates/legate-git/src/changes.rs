//! The Changes list: every orch worktree, named from the store, newest first.

use std::collections::HashMap;

use legate_core::{ChangeSet, SetupState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedName {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

pub fn merge_listed_changes(
    listed: &[ChangeSet],
    ids: &[String],
    names: &[ListedName],
    folder: impl Fn(&str) -> String,
    setup: impl Fn(&str) -> Option<SetupState>,
) -> Vec<ChangeSet> {
    let mut by_id: HashMap<&str, &ChangeSet> = HashMap::new();
    for change in listed {
        by_id.insert(change.id.as_str(), change);
    }
    let mut name_by_id: HashMap<&str, &ListedName> = HashMap::new();
    for row in names {
        name_by_id.insert(row.id.as_str(), row);
    }
    let mut merged = Vec::with_capacity(ids.len());
    for id in ids {
        let mut entry = if let Some(existing) = by_id.get(id.as_str()) {
            (*existing).clone()
        } else {
            ChangeSet {
                id: id.clone(),
                branch: format!("orch/{id}"),
                path: folder(id),
                files: Vec::new(),
                insertions: 0,
                deletions: 0,
                name: None,
                setup: None,
            }
        };
        if let Some(row) = name_by_id.get(id.as_str())
            && !row.name.is_empty()
        {
            entry.name = Some(row.name.clone());
        }
        if let Some(state) = setup(id) {
            entry.setup = Some(state);
        }
        merged.push(entry);
    }
    merged.sort_by(|a, b| {
        let ta = name_by_id
            .get(a.id.as_str())
            .map(|row| row.created_at)
            .unwrap_or(0);
        let tb = name_by_id
            .get(b.id.as_str())
            .map(|row| row.created_at)
            .unwrap_or(0);
        tb.cmp(&ta).then_with(|| a.id.cmp(&b.id))
    });
    merged
}
