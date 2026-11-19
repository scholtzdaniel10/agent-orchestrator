//! Browser smoke test for the whole app: mounts `App` against a stand-in for the Tauri bridge
//! and opens every screen the way a new user would. Setup on first run, each panel in
//! `DrawerTab::ALL`, each Settings section, and the New task dialog. A screen fails if it
//! panics (the step times out with the app's text) or if closing it leaves a Tauri listener
//! behind. Panels and sections come from the crate itself, so new ones are covered.
//!
//! Runs in a headless browser: see "Browser tests" in CONTRIBUTING.md.
#![cfg(target_arch = "wasm32")]

mod common;

use common::{
    assert_no_app_panic, click, exists, find, install_bridge, listener_counts, mount, settle,
    wait_for,
};
use legate_core::{AppInfo, BranchStatus, FileList, ProjectInfo, SkillsState};
use legate_ui::app::App;
use legate_ui::logic::layout::DrawerTab;
use legate_ui::state::default_prefs;
use legate_ui::wire::CommitPage;
use serde_json::{Map, Value, json, to_value};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

const ROOT: &str = "app-smoke-root";

/// An empty, signed-out project. Commands not listed resolve to `null`, which an `Option` or
/// `()` reads as nothing and anything else reports as an error the screen shows.
fn answers() -> Map<String, Value> {
    let empty_lists = [
        "list_jobs",
        "list_plans",
        "list_projects",
        "list_changes",
        "list_goals",
        "list_triggers",
        "list_routines",
        "list_lead_messages",
        "list_lead_chats",
        "list_terminals",
        "list_project_terminals",
        "restore_terminals",
        "popped_out",
        "popped_panels",
        "get_project_setup",
        "setup_status",
        "github_repos",
    ];
    let mut answers: Map<String, Value> = empty_lists
        .iter()
        .map(|cmd| (cmd.to_string(), json!([])))
        .collect();
    let project = ProjectInfo {
        path: "C:/fixture".into(),
        is_repo: true,
        branch: Some("main".into()),
    };
    let info = AppInfo {
        version: "0.0.0-test".into(),
        data_dir: "C:/data".into(),
        worktrees_dir: "C:/data/worktrees".into(),
        rules_max_parallel: 2,
    };
    let files = FileList {
        entries: Vec::new(),
        truncated: false,
    };
    let skills = SkillsState {
        built_in: Vec::new(),
        folders: Vec::new(),
    };
    let commits = CommitPage {
        commits: Vec::new(),
        more: false,
    };
    let branch = BranchStatus {
        branch: "main".into(),
        upstream: None,
        base: None,
        ahead: 0,
        behind: 0,
        uncommitted: Vec::new(),
        commits: 0,
        insertions: 0,
        deletions: 0,
    };
    for (cmd, value) in [
        ("get_project", to_value(project)),
        ("get_prefs", to_value(default_prefs())),
        ("get_app_info", to_value(info)),
        ("list_files", to_value(files)),
        ("skills_get", to_value(skills)),
        ("commit_log", to_value(commits)),
        ("branch_status", to_value(branch)),
        ("github_available", Ok(json!(false))),
    ] {
        answers.insert(cmd.into(), value.expect("an answer serializes"));
    }
    answers
}

/// Fails `screen` if closing it changed which Tauri listeners are live, naming each event
/// whose count changed. The inspector mounts every panel at once, so a panel leak shows on
/// the first panel step; the event name says which listener it is.
fn assert_no_leftover_listeners(screen: &str, baseline: &str) {
    let now = listener_counts();
    if now == baseline {
        return;
    }
    let before: Map<String, Value> = serde_json::from_str(baseline).unwrap_or_default();
    let after: Map<String, Value> = serde_json::from_str(&now).unwrap_or_default();
    let mut names: Vec<&String> = before.keys().chain(after.keys()).collect();
    names.sort();
    names.dedup();
    let changed: Vec<String> = names
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .map(|name| {
            let count =
                |map: &Map<String, Value>| map.get(name).and_then(Value::as_u64).unwrap_or(0);
            format!("{name} {} -> {}", count(&before), count(&after))
        })
        .collect();
    panic!(
        "After opening and closing {screen}, these Tauri listeners did not go back: {}. \
         Some component held them past unmount (the inspector mounts every panel at once).",
        changed.join(", ")
    );
}

async fn close_inspector() {
    if exists(".drawer") {
        click(ROOT, "[aria-label=\"Close inspector\"]", None).await;
        wait_for(ROOT, "the inspector to close", || !exists(".drawer")).await;
    }
}

#[wasm_bindgen_test]
async fn every_screen_opens_and_closes_cleanly() {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.clear();
    }
    install_bridge(answers());
    mount(ROOT, App);

    // First run: Setup shows over the workspace.
    wait_for(ROOT, "Setup on first run", || exists(".setup-screen")).await;
    click(ROOT, ".setup-footer button", Some("Skip for now")).await;
    wait_for(ROOT, "Setup to close", || !exists(".setup-screen")).await;
    wait_for(ROOT, "the workspace", || exists(".command-rail")).await;
    close_inspector().await;
    settle().await;
    let baseline = listener_counts();

    for tab in DrawerTab::ALL {
        let label = tab.label();
        let screen = format!("the {label} panel");
        click(
            ROOT,
            &format!(".command-rail button[title=\"{label}\"]"),
            None,
        )
        .await;
        wait_for(ROOT, &format!("{screen} to open"), || {
            find(".drawer .inspector-heading h2", Some(label)).is_some()
        })
        .await;
        settle().await;
        if tab == DrawerTab::Source {
            wait_for(ROOT, "the current branch", || {
                find(".drawer .files-status", Some("main ↑0 ↓0")).is_some()
            })
            .await;
        }
        close_inspector().await;
        settle().await;
        assert_no_leftover_listeners(&screen, &baseline);
    }

    click(ROOT, ".command-settings", None).await;
    wait_for(ROOT, "Settings to open", || exists(".settings-page")).await;
    let sections: Vec<String> = {
        let items = common::document()
            .query_selector_all(".settings-nav-item")
            .expect("settings sections");
        (0..items.length())
            .filter_map(|at| items.item(at)?.text_content())
            .map(|text| text.trim().to_string())
            .collect()
    };
    assert!(!sections.is_empty(), "Settings lists no sections.");
    for section in &sections {
        click(ROOT, ".settings-nav-item", Some(section)).await;
        wait_for(ROOT, &format!("the {section} settings"), || {
            find(".settings-nav-item.is-selected", Some(section)).is_some()
        })
        .await;
        settle().await;
    }
    click(ROOT, ".settings-head button", Some("Close")).await;
    wait_for(ROOT, "Settings to close", || !exists(".settings-page")).await;
    settle().await;
    assert_no_leftover_listeners("Settings", &baseline);

    click(ROOT, ".command-new-task", None).await;
    wait_for(ROOT, "the New task dialog", || exists(".new-task-overlay")).await;
    click(ROOT, ".new-task-actions button", Some("Cancel")).await;
    wait_for(ROOT, "the New task dialog to close", || {
        !exists(".new-task-overlay")
    })
    .await;
    settle().await;
    assert_no_leftover_listeners("the New task dialog", &baseline);
    assert_no_app_panic("the walk through every screen");
}
