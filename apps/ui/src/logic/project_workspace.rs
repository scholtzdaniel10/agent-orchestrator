//! Per-project workspace snapshots. Provider processes are owned by the backend.
use super::layout::LayoutState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectWorkspace {
    pub layout: LayoutState,
    pub selected: Vec<Option<String>>,
    pub primary: Option<String>,
    pub terminal: String,
}

pub fn storage_key(project: &str, kind: &str) -> String {
    format!("legate.project.v1:{kind}:{project}")
}

#[cfg(target_arch = "wasm32")]
pub fn read<T: serde::de::DeserializeOwned>(project: &str, kind: &str) -> Option<T> {
    let storage = web_sys::window()?.local_storage().ok()??;
    serde_json::from_str(&storage.get_item(&storage_key(project, kind)).ok()??).ok()
}
#[cfg(target_arch = "wasm32")]
pub fn write<T: Serialize>(project: &str, kind: &str, value: &T) {
    if let (Some(storage), Ok(raw)) = (
        web_sys::window().and_then(|w| w.local_storage().ok().flatten()),
        serde_json::to_string(value),
    ) {
        let _ = storage.set_item(&storage_key(project, kind), &raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn different_projects_and_state_kinds_do_not_share_keys() {
        assert_ne!(storage_key("C:/a", "layout"), storage_key("C:/b", "layout"));
        assert_ne!(storage_key("C:/a", "layout"), storage_key("C:/a", "draft"));
    }
    #[test]
    fn custom_docks_and_selection_round_trip_without_becoming_defaults() {
        let mut layout = super::super::layout::default_layout();
        layout.dock_tabs.left = vec![super::super::layout::DrawerTab::Lead];
        layout.drawer_open = false;
        let state = ProjectWorkspace {
            layout,
            selected: vec![Some("branch-a".into())],
            primary: Some("branch-a".into()),
            terminal: "term-a".into(),
        };
        let raw = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<ProjectWorkspace>(&raw).unwrap(),
            state
        );
    }
}

/// A rejected request must not overwrite text the user composed while waiting.
pub fn restore_failed_draft(current: &str, failed: &str) -> String {
    if current.is_empty() {
        failed.to_string()
    } else {
        current.to_string()
    }
}

#[cfg(test)]
mod draft_tests {
    #[test]
    fn a_failed_send_preserves_newer_unsent_text() {
        assert_eq!(
            super::restore_failed_draft("next task", "failed task"),
            "next task"
        );
        assert_eq!(
            super::restore_failed_draft("", "failed task"),
            "failed task"
        );
    }
}
